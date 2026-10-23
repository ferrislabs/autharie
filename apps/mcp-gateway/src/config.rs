use std::{collections::HashSet, fmt, net::SocketAddr};

use serde::Deserialize;
use url::Url;

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdpConfig {
    pub evaluation_url: String,
    pub evaluations_url: String,
    pub client_id: String,
    pub client_secret: Secret,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExchangeConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Secret,
    pub audience: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Egress {
    #[default]
    Cluster,
    Public,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamConfig {
    pub canonical_uri: String,
    pub issuer: String,
    #[serde(default)]
    pub jwks_uri: Option<String>,
    pub pdp: PdpConfig,
    pub exchange: ExchangeConfig,
    pub upstream_url: String,
    pub server_name: String,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub egress: Egress,
    #[serde(default)]
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default = "default_jwks_refresh_secs")]
    pub jwks_refresh_secs: u64,
    #[serde(default = "default_pdp_timeout_ms")]
    pub pdp_timeout_ms: u64,
    #[serde(default)]
    pub upstreams: Vec<UpstreamConfig>,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 8080))
}

fn default_jwks_refresh_secs() -> u64 {
    300
}

fn default_pdp_timeout_ms() -> u64 {
    2000
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("configuration unreadable: {0}")]
    Parse(String),
    #[error("upstream {server}: {field} is not a valid {expected}")]
    Invalid {
        server: String,
        field: &'static str,
        expected: &'static str,
    },
    #[error("upstream {0}: server_name must not be empty")]
    EmptyServerName(String),
    #[error("two upstreams share the canonical uri {0}")]
    Duplicate(String),
}

impl GatewayConfig {
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let config: Self =
            toml::from_str(source).map_err(|error| ConfigError::Parse(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        for upstream in &self.upstreams {
            upstream.validate()?;
            let key = upstream.canonical_uri.trim_end_matches('/').to_lowercase();
            if !seen.insert(key) {
                return Err(ConfigError::Duplicate(upstream.canonical_uri.clone()));
            }
        }
        Ok(())
    }
}

impl UpstreamConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let server = &self.server_name;
        if server.trim().is_empty() {
            return Err(ConfigError::EmptyServerName(self.canonical_uri.clone()));
        }
        let invalid = |field: &'static str, expected: &'static str| ConfigError::Invalid {
            server: server.clone(),
            field,
            expected,
        };
        let canonical_ok = Url::parse(&self.canonical_uri).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.query().is_none()
                && u.fragment().is_none()
        });
        if !canonical_ok {
            return Err(invalid(
                "canonical_uri",
                "absolute http(s) URI without query",
            ));
        }
        let urls = [
            ("issuer", &self.issuer),
            ("upstream_url", &self.upstream_url),
            ("pdp.evaluation_url", &self.pdp.evaluation_url),
            ("pdp.evaluations_url", &self.pdp.evaluations_url),
            ("exchange.token_url", &self.exchange.token_url),
        ];
        for (field, value) in urls {
            if !is_http_url(value) {
                return Err(invalid(field, "http(s) URL"));
            }
        }
        if let Some(jwks) = &self.jwks_uri
            && !is_http_url(jwks)
        {
            return Err(invalid("jwks_uri", "http(s) URL"));
        }
        if self.exchange.audience.trim().is_empty() {
            return Err(invalid("exchange.audience", "non-empty audience"));
        }
        Ok(())
    }

    pub fn jwks_url(&self) -> String {
        self.jwks_uri.clone().unwrap_or_else(|| {
            format!(
                "{}/protocol/openid-connect/certs",
                self.issuer.trim_end_matches('/')
            )
        })
    }
}

fn is_http_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r#"
        [[upstreams]]
        canonical_uri = "https://app.example/mcp"
        issuer = "https://auth.example/realms/autharie"
        upstream_url = "http://autharie-mcp:8080/mcp"
        server_name = "autharie"
        scopes_supported = ["autharie.read"]

        [upstreams.pdp]
        evaluation_url = "https://auth.example/api/realms/autharie/authzen/access/v1/evaluation"
        evaluations_url = "https://auth.example/api/realms/autharie/authzen/access/v1/evaluations"
        client_id = "mcp-gateway-pdp"
        client_secret = "pdp-secret"

        [upstreams.exchange]
        token_url = "https://auth.example/realms/autharie/protocol/openid-connect/token"
        client_id = "mcp-gateway"
        client_secret = "ex-secret"
        audience = "autharie-api"
    "#;

    #[test]
    fn a_frozen_contract_upstream_parses_with_defaults() {
        let config = GatewayConfig::from_toml(ONE).unwrap();
        assert_eq!(config.listen.port(), 8080);
        assert_eq!(config.pdp_timeout_ms, 2000);
        let upstream = &config.upstreams[0];
        assert_eq!(upstream.server_name, "autharie");
        assert_eq!(upstream.exchange.audience, "autharie-api");
        assert_eq!(upstream.pdp.client_secret.expose(), "pdp-secret");
        assert_eq!(upstream.scopes_supported, ["autharie.read"]);
    }

    #[test]
    fn the_jwks_uri_defaults_to_the_issuers_certs_endpoint() {
        let config = GatewayConfig::from_toml(ONE).unwrap();
        assert_eq!(
            config.upstreams[0].jwks_url(),
            "https://auth.example/realms/autharie/protocol/openid-connect/certs"
        );
        let explicit = ONE.replace(
            "server_name = \"autharie\"",
            "server_name = \"autharie\"\njwks_uri = \"https://keys.example/jwks\"",
        );
        let config = GatewayConfig::from_toml(&explicit).unwrap();
        assert_eq!(config.upstreams[0].jwks_url(), "https://keys.example/jwks");
    }

    #[test]
    fn secrets_never_print() {
        let config = GatewayConfig::from_toml(ONE).unwrap();
        let printed = format!("{config:?}");
        assert!(!printed.contains("pdp-secret"));
        assert!(!printed.contains("ex-secret"));
    }

    #[test]
    fn no_upstream_is_valid() {
        assert!(
            GatewayConfig::from_toml("upstreams = []")
                .unwrap()
                .upstreams
                .is_empty()
        );
        assert!(GatewayConfig::from_toml("").unwrap().upstreams.is_empty());
    }

    #[test]
    fn a_missing_field_is_refused() {
        let broken = ONE.replace("audience = \"autharie-api\"", "");
        assert!(matches!(
            GatewayConfig::from_toml(&broken),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn a_canonical_uri_with_a_query_is_refused() {
        let broken = ONE.replace("/mcp\"", "/mcp?x=1\"");
        assert!(matches!(
            GatewayConfig::from_toml(&broken),
            Err(ConfigError::Invalid {
                field: "canonical_uri",
                ..
            })
        ));
    }

    #[test]
    fn a_non_http_pdp_url_is_refused() {
        let broken = ONE.replace(
            "https://auth.example/api/realms/autharie/authzen/access/v1/evaluations",
            "ftp://auth.example/x",
        );
        assert!(matches!(
            GatewayConfig::from_toml(&broken),
            Err(ConfigError::Invalid {
                field: "pdp.evaluations_url",
                ..
            })
        ));
    }

    #[test]
    fn two_upstreams_cannot_claim_the_same_resource() {
        let doubled = format!("{ONE}\n{ONE}");
        let error = GatewayConfig::from_toml(&doubled).unwrap_err();
        assert!(matches!(error, ConfigError::Duplicate(_)));
    }

    #[test]
    fn an_upstream_reaches_the_cluster_unless_it_says_public() {
        let config = GatewayConfig::from_toml(ONE).unwrap();
        assert_eq!(config.upstreams[0].egress, Egress::Cluster);

        let public = ONE.replace(
            "server_name = \"autharie\"",
            "server_name = \"autharie\"\negress = \"public\"",
        );
        let config = GatewayConfig::from_toml(&public).unwrap();
        assert_eq!(config.upstreams[0].egress, Egress::Public);
    }

    #[test]
    fn an_upstream_speaks_2026_unless_it_names_another_protocol() {
        let config = GatewayConfig::from_toml(ONE).unwrap();
        assert_eq!(config.upstreams[0].protocol, None);

        let legacy = ONE.replace(
            "server_name = \"autharie\"",
            "server_name = \"autharie\"\nprotocol = \"2025-11-25\"",
        );
        let config = GatewayConfig::from_toml(&legacy).unwrap();
        assert_eq!(config.upstreams[0].protocol.as_deref(), Some("2025-11-25"));
    }

    #[test]
    fn an_egress_that_is_neither_cluster_nor_public_is_refused() {
        let broken = ONE.replace(
            "server_name = \"autharie\"",
            "server_name = \"autharie\"\negress = \"anywhere\"",
        );

        assert!(matches!(
            GatewayConfig::from_toml(&broken),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn an_empty_server_name_is_refused() {
        let broken = ONE.replace("server_name = \"autharie\"", "server_name = \" \"");
        assert!(matches!(
            GatewayConfig::from_toml(&broken),
            Err(ConfigError::EmptyServerName(_))
        ));
    }
}
