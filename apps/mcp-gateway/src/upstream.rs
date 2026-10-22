use std::{collections::HashMap, sync::Mutex, time::Duration};

use reqwest::Client;
use serde_json::Value;
use url::Url;

use crate::{
    authzen::AuthzenPdp,
    config::UpstreamConfig,
    exchange::{CachedExchanger, TokenExchanger},
    verifier::JwksVerifier,
};

pub const METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

pub struct Upstream {
    pub server_name: String,
    pub canonical_uri: String,
    pub issuer: String,
    pub scopes_supported: Vec<String>,
    pub upstream_url: String,
    pub verifier: JwksVerifier,
    pub pdp: AuthzenPdp,
    pub exchanger: CachedExchanger,
    host: String,
    port: Option<u16>,
    path: String,
    metadata_url: String,
    metadata_path: String,
    annotations: Mutex<HashMap<String, Value>>,
}

pub struct Timing {
    pub jwks_refresh: Duration,
    pub pdp_timeout: Duration,
    pub unknown_kid_cooldown: Duration,
}

impl Upstream {
    pub fn new(
        config: UpstreamConfig,
        http: &Client,
        timing: &Timing,
    ) -> Result<Self, url::ParseError> {
        let canonical = Url::parse(&config.canonical_uri)?;
        let path = canonical.path().trim_end_matches('/').to_owned();
        let metadata_path = format!("{METADATA_PATH}{path}");
        let metadata_url = format!(
            "{}{metadata_path}",
            canonical.origin().ascii_serialization()
        );
        let verifier = JwksVerifier::new(
            http.clone(),
            &config.issuer,
            config.canonical_uri.clone(),
            config.jwks_url(),
            timing.jwks_refresh,
        )
        .with_unknown_kid_cooldown(timing.unknown_kid_cooldown);
        let pdp = AuthzenPdp::new(
            http.clone(),
            config.pdp.clone(),
            config.exchange.token_url.clone(),
            timing.pdp_timeout,
        );
        let exchanger = CachedExchanger::new(TokenExchanger::new(http.clone(), config.exchange));
        Ok(Self {
            server_name: config.server_name,
            canonical_uri: config.canonical_uri,
            issuer: config.issuer,
            scopes_supported: config.scopes_supported,
            upstream_url: config.upstream_url,
            verifier,
            pdp,
            exchanger,
            host: canonical.host_str().unwrap_or_default().to_lowercase(),
            port: canonical.port(),
            path,
            metadata_url,
            metadata_path,
            annotations: Mutex::new(HashMap::new()),
        })
    }

    pub fn metadata_url(&self) -> &str {
        &self.metadata_url
    }

    pub fn challenge(&self, invalid_token: bool) -> String {
        let mut challenge = String::from("Bearer");
        let mut parts = Vec::new();
        if invalid_token {
            parts.push(String::from("error=\"invalid_token\""));
        }
        parts.push(format!("resource_metadata=\"{}\"", self.metadata_url));
        if !self.scopes_supported.is_empty() {
            parts.push(format!("scope=\"{}\"", self.scopes_supported.join(" ")));
        }
        challenge.push(' ');
        challenge.push_str(&parts.join(", "));
        challenge
    }

    pub fn metadata(&self) -> Value {
        serde_json::json!({
            "resource": self.canonical_uri,
            "authorization_servers": [self.issuer],
            "scopes_supported": self.scopes_supported,
            "bearer_methods_supported": ["header"],
        })
    }

    fn matches_host(&self, host: &str) -> bool {
        let (name, port) = split_host(host);
        name.eq_ignore_ascii_case(&self.host) && self.port.is_none_or(|p| port == Some(p))
    }

    fn serves_path(&self, path: &str) -> bool {
        match path.strip_prefix(self.path.as_str()) {
            Some("") => true,
            Some(rest) => rest.starts_with('/'),
            None => false,
        }
    }

    pub fn remember_annotations(&self, tool: &str, annotations: &Value) {
        self.annotations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tool.to_owned(), annotations.clone());
    }

    pub fn annotations_of(&self, tool: &str) -> Value {
        self.annotations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(tool)
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()))
    }
}

fn split_host(host: &str) -> (&str, Option<u16>) {
    match host.rsplit_once(':') {
        Some((name, port)) if !host.ends_with(']') => match port.parse() {
            Ok(port) => (name, Some(port)),
            Err(_) => (host, None),
        },
        _ => (host, None),
    }
}

pub struct Routes {
    upstreams: Vec<Upstream>,
}

impl Routes {
    pub fn new(upstreams: Vec<Upstream>) -> Self {
        Self { upstreams }
    }

    pub fn for_resource(&self, host: &str, path: &str) -> Option<&Upstream> {
        self.upstreams
            .iter()
            .filter(|u| u.matches_host(host) && u.serves_path(path))
            .max_by_key(|u| u.path.len())
    }

    pub fn for_metadata(&self, host: &str, path: &str) -> Option<&Upstream> {
        self.upstreams
            .iter()
            .find(|u| u.matches_host(host) && u.metadata_path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;

    fn routes(uris: &[&str]) -> Routes {
        let upstreams = uris
            .iter()
            .enumerate()
            .map(|(i, uri)| {
                let source = format!(
                    r#"
                    [[upstreams]]
                    canonical_uri = "{uri}"
                    issuer = "https://auth.example/realms/r"
                    upstream_url = "http://up-{i}/mcp"
                    server_name = "s{i}"
                    scopes_supported = ["a.read", "a.write"]
                    [upstreams.pdp]
                    evaluation_url = "http://pdp/e"
                    evaluations_url = "http://pdp/es"
                    client_id = "c"
                    client_secret = "s"
                    [upstreams.exchange]
                    token_url = "http://auth/token"
                    client_id = "c"
                    client_secret = "s"
                    audience = "api"
                    "#
                );
                GatewayConfig::from_toml(&source)
                    .unwrap()
                    .upstreams
                    .remove(0)
            })
            .collect::<Vec<_>>();
        let http = Client::new();
        let timing = Timing {
            jwks_refresh: Duration::from_secs(1),
            pdp_timeout: Duration::from_secs(1),
            unknown_kid_cooldown: Duration::ZERO,
        };
        Routes::new(
            upstreams
                .into_iter()
                .map(|u| Upstream::new(u, &http, &timing).unwrap())
                .collect(),
        )
    }

    #[test]
    fn an_upstream_is_selected_by_host_and_path() {
        let routes = routes(&["https://app.example/mcp", "https://db.example/mcp"]);
        assert_eq!(
            routes
                .for_resource("app.example", "/mcp")
                .unwrap()
                .server_name,
            "s0"
        );
        assert_eq!(
            routes
                .for_resource("DB.example:8443", "/mcp")
                .unwrap()
                .server_name,
            "s1"
        );
        assert!(routes.for_resource("other.example", "/mcp").is_none());
        assert!(routes.for_resource("app.example", "/other").is_none());
        assert!(routes.for_resource("app.example", "/mcpx").is_none());
    }

    #[test]
    fn the_longest_path_wins_on_a_shared_host() {
        let routes = routes(&["https://x.example/mcp", "https://x.example/mcp/db"]);
        assert_eq!(
            routes
                .for_resource("x.example", "/mcp")
                .unwrap()
                .server_name,
            "s0"
        );
        assert_eq!(
            routes
                .for_resource("x.example", "/mcp/db")
                .unwrap()
                .server_name,
            "s1"
        );
        assert_eq!(
            routes
                .for_resource("x.example", "/mcp/db/x")
                .unwrap()
                .server_name,
            "s1"
        );
    }

    #[test]
    fn an_explicit_canonical_port_must_match() {
        let routes = routes(&["http://127.0.0.1:9000/mcp"]);
        assert!(routes.for_resource("127.0.0.1:9000", "/mcp").is_some());
        assert!(routes.for_resource("127.0.0.1:9001", "/mcp").is_none());
        assert!(routes.for_resource("127.0.0.1", "/mcp").is_none());
    }

    #[test]
    fn metadata_lives_at_the_well_known_path_with_the_resource_path_appended() {
        let routes = routes(&["https://app.example/mcp"]);
        let upstream = routes
            .for_metadata("app.example", "/.well-known/oauth-protected-resource/mcp")
            .unwrap();
        assert_eq!(
            upstream.metadata_url(),
            "https://app.example/.well-known/oauth-protected-resource/mcp"
        );
        assert!(
            routes
                .for_metadata("app.example", "/.well-known/oauth-protected-resource")
                .is_none()
        );
        assert_eq!(upstream.metadata()["resource"], "https://app.example/mcp");
    }

    #[test]
    fn the_challenge_names_the_metadata_and_the_scopes() {
        let routes = routes(&["https://app.example/mcp"]);
        let upstream = routes.for_resource("app.example", "/mcp").unwrap();
        assert_eq!(
            upstream.challenge(false),
            "Bearer resource_metadata=\"https://app.example/.well-known/oauth-protected-resource/mcp\", scope=\"a.read a.write\""
        );
        assert!(
            upstream
                .challenge(true)
                .starts_with("Bearer error=\"invalid_token\", resource_metadata=")
        );
    }

    #[test]
    fn annotations_are_remembered_per_tool() {
        let routes = routes(&["https://app.example/mcp"]);
        let upstream = routes.for_resource("app.example", "/mcp").unwrap();
        assert_eq!(upstream.annotations_of("t"), serde_json::json!({}));
        upstream.remember_annotations("t", &serde_json::json!({"readOnlyHint": true}));
        assert_eq!(
            upstream.annotations_of("t"),
            serde_json::json!({"readOnlyHint": true})
        );
    }
}
