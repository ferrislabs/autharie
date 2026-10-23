use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::{Host, Url};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{agents::AgentError, deployments::DeploymentId};

pub const MAX_SERVERS: usize = 10;
pub const MAX_DISCOVERED_TOOLS: usize = 200;
pub const MAX_TOOL_NAME_CHARS: usize = 64;
pub const MAX_TOOL_DESCRIPTION_CHARS: usize = 512;
pub const MAX_DISCOVERY_MESSAGE_CHARS: usize = 300;
pub const MAX_PROTOCOL_VERSION_CHARS: usize = 32;

const MAX_NAME_CHARS: usize = 100;
const MAX_URL_BYTES: usize = 2048;
const RESERVED_SLUGS: [&str; 3] = ["ferriskey", "autharie", "gateway"];
const INTERNAL_SUFFIXES: [&str; 8] = [
    ".svc",
    ".home.arpa",
    ".local",
    ".localhost",
    ".localdomain",
    ".internal",
    ".intranet",
    ".lan",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UrlRefusal {
    #[error("is not a URL")]
    Unparseable,
    #[error("is longer than {MAX_URL_BYTES} characters")]
    TooLong,
    #[error("must start with https://")]
    NotHttps,
    #[error("must not carry a user name or password")]
    Credentials,
    #[error("must not carry a fragment")]
    Fragment,
    #[error("must name a host")]
    NoHost,
    #[error("must name a public host, not an internal one")]
    InternalHost,
    #[error("must not point at a private, loopback or link-local address")]
    PrivateAddress,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServerError {
    #[error(
        "slug: must be 2 to 31 characters, start with a letter and hold lowercase letters, digits and hyphens"
    )]
    SlugShape,
    #[error("slug: '{0}' is reserved")]
    SlugReserved(String),
    #[error("name: must be 1 to {MAX_NAME_CHARS} characters")]
    Name,
    #[error("url: {0}")]
    Url(UrlRefusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, format = Uuid)]
pub struct ServerId(pub Uuid);

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct ServerSlug(String);

impl ServerSlug {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServerSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for ServerSlug {
    type Error = ServerError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut chars = value.chars();
        let well_formed = (2..=31).contains(&value.len())
            && chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !well_formed {
            return Err(ServerError::SlugShape);
        }
        if RESERVED_SLUGS.contains(&value) {
            return Err(ServerError::SlugReserved(value.to_string()));
        }

        Ok(Self(value.to_string()))
    }
}

impl TryFrom<String> for ServerSlug {
    type Error = ServerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct CustomerServerUrl(String);

impl CustomerServerUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CustomerServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for CustomerServerUrl {
    type Error = ServerError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let refuse = |reason| ServerError::Url(reason);

        let value = value.trim();
        if value.len() > MAX_URL_BYTES {
            return Err(refuse(UrlRefusal::TooLong));
        }
        let url = Url::parse(value).map_err(|_| refuse(UrlRefusal::Unparseable))?;
        if url.scheme() != "https" {
            return Err(refuse(UrlRefusal::NotHttps));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(refuse(UrlRefusal::Credentials));
        }
        if url.fragment().is_some() {
            return Err(refuse(UrlRefusal::Fragment));
        }
        match url.host() {
            None => return Err(refuse(UrlRefusal::NoHost)),
            Some(Host::Ipv4(address)) => {
                if !autharie_egress::is_public(address.into()) {
                    return Err(refuse(UrlRefusal::PrivateAddress));
                }
            }
            Some(Host::Ipv6(address)) => {
                if !autharie_egress::is_public(address.into()) {
                    return Err(refuse(UrlRefusal::PrivateAddress));
                }
            }
            Some(Host::Domain(name)) => {
                let name = name.trim_end_matches('.').to_ascii_lowercase();
                let internal = !name.contains('.')
                    || INTERNAL_SUFFIXES
                        .iter()
                        .any(|suffix| name.ends_with(suffix));
                if internal {
                    return Err(refuse(UrlRefusal::InternalHost));
                }
            }
        }

        Ok(Self(url.to_string()))
    }
}

impl TryFrom<String> for CustomerServerUrl {
    type Error = ServerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

pub fn parse_server_name(name: &str) -> Result<String, ServerError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(ServerError::Name);
    }

    Ok(name.to_string())
}

pub fn is_tool_name(name: &str) -> bool {
    (1..=MAX_TOOL_NAME_CHARS).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum DiscoveryPhase {
    Ready,
    Unreachable,
    Refused,
    Invalid,
}

impl DiscoveryPhase {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "Ready" => Some(Self::Ready),
            "Unreachable" => Some(Self::Unreachable),
            "Refused" => Some(Self::Refused),
            "Invalid" => Some(Self::Invalid),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Unreachable => "Unreachable",
            Self::Refused => "Refused",
            Self::Invalid => "Invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DiscoveredTool {
    pub name: String,
    pub description: Option<String>,
    pub read_only: bool,
    pub destructive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Discovery {
    pub phase: DiscoveryPhase,
    pub tools: Vec<DiscoveredTool>,
    pub dropped: u32,
    pub message: Option<String>,
    pub discovered_at: DateTime<Utc>,
    #[serde(default)]
    pub protocol_version: Option<String>,
}

fn cut(text: String, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => text[..end].to_string(),
        None => text,
    }
}

impl Discovery {
    pub fn bounded(
        phase: DiscoveryPhase,
        tools: Vec<DiscoveredTool>,
        dropped: u32,
        message: Option<String>,
        discovered_at: DateTime<Utc>,
    ) -> Self {
        let mut dropped = dropped;
        let mut kept: Vec<DiscoveredTool> = Vec::new();

        for tool in tools {
            if kept.len() >= MAX_DISCOVERED_TOOLS
                || !is_tool_name(&tool.name)
                || kept.iter().any(|known| known.name == tool.name)
            {
                dropped = dropped.saturating_add(1);
                continue;
            }
            kept.push(DiscoveredTool {
                description: tool
                    .description
                    .map(|text| cut(text, MAX_TOOL_DESCRIPTION_CHARS)),
                ..tool
            });
        }

        Self {
            phase,
            tools: kept,
            dropped,
            message: message
                .map(|text| cut(text, MAX_DISCOVERY_MESSAGE_CHARS))
                .filter(|text| !text.is_empty()),
            discovered_at,
            protocol_version: None,
        }
    }

    pub fn with_protocol_version(mut self, version: Option<String>) -> Self {
        self.protocol_version = version
            .map(|text| cut(text, MAX_PROTOCOL_VERSION_CHARS))
            .filter(|text| !text.is_empty());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CustomerServer {
    pub id: ServerId,
    #[serde(skip)]
    pub deployment_id: DeploymentId,
    pub slug: ServerSlug,
    pub name: String,
    pub url: CustomerServerUrl,
    pub enabled: bool,
    pub mcp_url: Option<String>,
    pub discovery: Option<Discovery>,
    #[serde(skip)]
    pub discovery_requested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CustomerServer {
    pub fn with_mcp_url(mut self, gateway_url: Option<&str>) -> Self {
        self.mcp_url =
            gateway_url.map(|base| format!("{}/{}", base.trim_end_matches('/'), self.slug));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInput {
    pub slug: Option<ServerSlug>,
    pub name: String,
    pub url: CustomerServerUrl,
    pub enabled: bool,
}

impl ServerInput {
    pub fn register(
        slug: &str,
        name: &str,
        url: &str,
        enabled: Option<bool>,
    ) -> Result<Self, ServerError> {
        Ok(Self {
            slug: Some(ServerSlug::try_from(slug)?),
            name: parse_server_name(name)?,
            url: CustomerServerUrl::try_from(url)?,
            enabled: enabled.unwrap_or(true),
        })
    }

    pub fn change(name: &str, url: &str, enabled: bool) -> Result<Self, ServerError> {
        Ok(Self {
            slug: None,
            name: parse_server_name(name)?,
            url: CustomerServerUrl::try_from(url)?,
            enabled,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOutcome {
    pub id: ServerId,
    pub discovery: Option<Discovery>,
    pub answers: Option<DateTime<Utc>>,
}

impl ServerOutcome {
    pub fn with_protocol_version(mut self, version: Option<String>) -> Self {
        self.discovery = self
            .discovery
            .map(|discovery| discovery.with_protocol_version(version));
        self
    }

    pub fn parse(
        id: Uuid,
        phase: &str,
        tools: Vec<DiscoveredTool>,
        dropped: u32,
        message: Option<String>,
        discovered_at: Option<DateTime<Utc>>,
        answers: Option<DateTime<Utc>>,
    ) -> Result<Self, AgentError> {
        let phase = DiscoveryPhase::parse(phase).ok_or_else(|| {
            AgentError::invalid(format!("'{phase}' is not a server discovery phase"))
        })?;

        Ok(Self {
            id: ServerId(id),
            discovery: discovered_at
                .map(|at| Discovery::bounded(phase, tools, dropped, message, at)),
            answers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> DiscoveredTool {
        DiscoveredTool {
            name: name.to_string(),
            description: None,
            read_only: false,
            destructive: false,
        }
    }

    #[test]
    fn the_negotiated_protocol_is_kept_bounded_and_never_empty() {
        let discovery = || Discovery::bounded(DiscoveryPhase::Ready, vec![], 0, None, Utc::now());

        assert_eq!(discovery().protocol_version, None);
        assert_eq!(
            discovery()
                .with_protocol_version(Some("2025-11-25".to_string()))
                .protocol_version
                .as_deref(),
            Some("2025-11-25")
        );
        assert_eq!(
            discovery()
                .with_protocol_version(Some(String::new()))
                .protocol_version,
            None
        );
        let long = discovery().with_protocol_version(Some("v".repeat(500)));
        assert_eq!(long.protocol_version.unwrap().chars().count(), 32);
    }

    #[test]
    fn a_stored_discovery_without_a_protocol_still_reads() {
        let stored = serde_json::json!({
            "phase": "Ready", "tools": [], "dropped": 0, "message": null,
            "discovered_at": "2026-10-23T10:00:00Z",
        });

        let discovery: Discovery = serde_json::from_value(stored).unwrap();

        assert_eq!(discovery.protocol_version, None);
    }

    #[test]
    fn an_outcome_carries_the_protocol_into_its_discovery_only_when_it_has_one() {
        let id = Uuid::new_v4();
        let waiting = ServerOutcome::parse(id, "Ready", vec![], 0, None, None, None)
            .unwrap()
            .with_protocol_version(Some("2025-11-25".to_string()));
        assert!(waiting.discovery.is_none());
        let done = ServerOutcome::parse(id, "Ready", vec![], 0, None, Some(Utc::now()), None)
            .unwrap()
            .with_protocol_version(Some("2025-11-25".to_string()));
        assert_eq!(
            done.discovery.unwrap().protocol_version.as_deref(),
            Some("2025-11-25")
        );
    }

    #[test]
    fn a_public_https_url_is_accepted_as_written() {
        for text in [
            "https://tools.example.com/mcp",
            "https://tools.example.com:8443/mcp?tenant=a",
            "https://8.8.8.8/mcp",
            "https://[2606:4700:4700::1111]/mcp",
        ] {
            assert_eq!(CustomerServerUrl::try_from(text).unwrap().as_str(), text);
        }
    }

    #[test]
    fn plain_http_credentials_and_fragments_are_refused() {
        assert_eq!(
            CustomerServerUrl::try_from("http://tools.example.com/mcp"),
            Err(ServerError::Url(UrlRefusal::NotHttps))
        );
        assert_eq!(
            CustomerServerUrl::try_from("https://user:pw@x.example/"),
            Err(ServerError::Url(UrlRefusal::Credentials))
        );
        assert_eq!(
            CustomerServerUrl::try_from("https://user@x.example/"),
            Err(ServerError::Url(UrlRefusal::Credentials))
        );
        assert_eq!(
            CustomerServerUrl::try_from("https://x.example/#top"),
            Err(ServerError::Url(UrlRefusal::Fragment))
        );
        assert_eq!(
            CustomerServerUrl::try_from("not a url"),
            Err(ServerError::Url(UrlRefusal::Unparseable))
        );
    }

    #[test]
    fn private_loopback_link_local_and_metadata_addresses_are_refused() {
        for text in [
            "https://10.0.0.5/mcp",
            "https://127.0.0.1/",
            "https://169.254.169.254/latest",
            "https://192.168.1.10:8443/",
            "https://100.64.0.1/",
            "https://[::1]/",
            "https://[fd00:ec2::254]/",
            "https://[::ffff:10.0.0.1]/",
            "https://0x7f.1/",
            "https://2130706433/",
        ] {
            assert_eq!(
                CustomerServerUrl::try_from(text),
                Err(ServerError::Url(UrlRefusal::PrivateAddress)),
                "{text}"
            );
        }
    }

    #[test]
    fn hosts_that_only_make_sense_inside_a_network_are_refused() {
        for text in [
            "https://localhost/mcp",
            "https://autharie-mcp/mcp",
            "https://crm.local/",
            "https://crm.internal/",
            "https://api.default.svc.cluster.local/",
            "https://ferriskey-mcp.autharie-system.svc/mcp",
            "https://nas.home.arpa/",
            "https://printer.lan./",
        ] {
            let refused = CustomerServerUrl::try_from(text);
            assert!(
                matches!(refused, Err(ServerError::Url(UrlRefusal::InternalHost))),
                "{text}: {refused:?}"
            );
        }
    }

    #[test]
    fn an_overlong_url_is_refused() {
        let long = format!("https://x.example/{}", "a".repeat(2100));

        assert_eq!(
            CustomerServerUrl::try_from(long.as_str()),
            Err(ServerError::Url(UrlRefusal::TooLong))
        );
    }

    #[test]
    fn a_slug_is_a_short_lowercase_name_that_starts_with_a_letter() {
        for good in ["crm", "billing-v2", "ab", &"a".repeat(31)] {
            assert_eq!(ServerSlug::try_from(good).unwrap().as_str(), good);
        }
        for bad in [
            "",
            "a",
            "1crm",
            "-crm",
            "CRM",
            "crm_x",
            "crm.x",
            "crm/x",
            "crm x",
            &"a".repeat(32),
        ] {
            assert_eq!(
                ServerSlug::try_from(bad),
                Err(ServerError::SlugShape),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_names_of_the_built_in_servers_and_of_the_gateway_client_are_reserved() {
        for reserved in ["ferriskey", "autharie", "gateway"] {
            assert_eq!(
                ServerSlug::try_from(reserved),
                Err(ServerError::SlugReserved(reserved.to_string()))
            );
        }
    }

    #[test]
    fn a_server_name_is_trimmed_and_bounded() {
        assert_eq!(parse_server_name("  CRM  ").unwrap(), "CRM");
        assert_eq!(parse_server_name(" "), Err(ServerError::Name));
        assert_eq!(parse_server_name(&"n".repeat(101)), Err(ServerError::Name));
    }

    #[test]
    fn every_refusal_names_its_field() {
        let url = ServerInput::register("crm", "CRM", "http://x.example/", None).unwrap_err();
        let slug = ServerInput::register("Bad", "CRM", "https://x.example/", None).unwrap_err();
        let name = ServerInput::register("crm", " ", "https://x.example/", None).unwrap_err();

        assert!(url.to_string().starts_with("url:"));
        assert!(slug.to_string().starts_with("slug:"));
        assert!(name.to_string().starts_with("name:"));
    }

    #[test]
    fn a_registered_server_is_enabled_unless_said_otherwise() {
        let on = ServerInput::register("crm", "CRM", "https://x.example/mcp", None).unwrap();
        let off =
            ServerInput::register("crm", "CRM", "https://x.example/mcp", Some(false)).unwrap();

        assert!(on.enabled);
        assert!(!off.enabled);
    }

    #[test]
    fn discovery_keeps_at_most_two_hundred_tools_and_counts_the_rest() {
        let tools = (0..500).map(|i| tool(&format!("t{i}"))).collect();

        let discovery = Discovery::bounded(DiscoveryPhase::Ready, tools, 0, None, Utc::now());

        assert_eq!(discovery.tools.len(), 200);
        assert_eq!(discovery.dropped, 300);
    }

    #[test]
    fn discovery_drops_names_outside_the_allowed_alphabet_and_counts_them() {
        let tools = vec![
            tool("list_invoices"),
            tool("bad name"),
            tool("a/b"),
            tool(""),
            tool(&"x".repeat(65)),
            tool("ok.v2-beta"),
            tool("list_invoices"),
        ];

        let discovery = Discovery::bounded(DiscoveryPhase::Ready, tools, 2, None, Utc::now());

        let names: Vec<_> = discovery.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["list_invoices", "ok.v2-beta"]);
        assert_eq!(discovery.dropped, 2 + 5);
    }

    #[test]
    fn discovery_cuts_descriptions_to_512_characters() {
        let long = DiscoveredTool {
            description: Some("é".repeat(600)),
            ..tool("t")
        };

        let discovery = Discovery::bounded(DiscoveryPhase::Ready, vec![long], 0, None, Utc::now());

        assert_eq!(
            discovery.tools[0]
                .description
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            512
        );
    }

    #[test]
    fn an_outcome_names_a_known_phase_and_carries_nothing_before_a_discovery() {
        let id = Uuid::from_u128(5);

        assert!(
            ServerOutcome::parse(id, "Exploded", vec![], 0, None, Some(Utc::now()), None).is_err()
        );
        let waiting = ServerOutcome::parse(id, "Ready", vec![], 0, None, None, None).unwrap();
        assert_eq!(waiting.discovery, None);
        let done = ServerOutcome::parse(
            id,
            "Unreachable",
            vec![],
            0,
            Some("timed out".into()),
            Some(Utc::now()),
            None,
        )
        .unwrap();
        assert_eq!(done.discovery.unwrap().phase, DiscoveryPhase::Unreachable);
    }

    #[test]
    fn the_gateway_url_of_a_server_extends_the_deployments_mcp_url() {
        let server = CustomerServer {
            id: ServerId(Uuid::nil()),
            deployment_id: DeploymentId(Uuid::nil()),
            slug: ServerSlug::try_from("crm").unwrap(),
            name: "CRM".into(),
            url: CustomerServerUrl::try_from("https://x.example/mcp").unwrap(),
            enabled: true,
            mcp_url: None,
            discovery: None,
            discovery_requested_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(
            server
                .clone()
                .with_mcp_url(Some("https://auth.acme.io/mcp"))
                .mcp_url
                .as_deref(),
            Some("https://auth.acme.io/mcp/crm")
        );
        assert_eq!(server.with_mcp_url(None).mcp_url, None);
    }
}
