pub mod config;
pub mod plan;
pub mod ports;
pub mod service;

use autharie_crds::v1alpha::identity_instance::{
    AgentPolicySpec, AgentsPhase, AgentsStatus, IdentityInstance,
};
use uuid::Uuid;

pub const MANAGED_BY: &str = "autharie";
pub const MANAGED_PREFIX: &str = "autharie:";
pub const GATEWAY_CLIENT_ID: &str = "mcp-gateway";
pub const GATEWAY_CONFIG_SECRET: &str = "mcp-gateway-config";
pub const GATEWAY_CONFIG_KEY: &str = "config.toml";
pub const FERRISKEY_SERVER_NAME: &str = "ferriskey";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentsError {
    Retry { message: String },
    Rejected { message: String },
    NotFound,
}

impl AgentsError {
    pub fn message(&self) -> &str {
        match self {
            Self::Retry { message } | Self::Rejected { message } => message,
            Self::NotFound => "FerrisKey does not have what was asked for",
        }
    }

    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePolicy {
    pub ferriskey_id: String,
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPolicy {
    pub autharie_id: String,
    pub policy: RemotePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyWrite {
    pub name: String,
    pub description: String,
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RealmSettings {
    pub allowed_resources: Vec<String>,
    pub cimd_enabled: bool,
    pub dcr_enabled: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct GatewayClient {
    pub secret: String,
}

impl std::fmt::Debug for GatewayClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayClient")
            .field("secret", &"<redacted>")
            .finish()
    }
}

pub fn managed_name(autharie_id: &str) -> String {
    format!("{MANAGED_PREFIX}{autharie_id}")
}

pub fn managed_id(name: &str) -> Option<&str> {
    let id = name.strip_prefix(MANAGED_PREFIX)?;
    Uuid::parse_str(id).ok()?;
    Some(id)
}

pub fn remote_description(spec: &AgentPolicySpec) -> String {
    spec.name.trim().to_string()
}

pub fn evaluator_audience(realm: &str) -> String {
    format!("{realm}-realm")
}

pub fn agents_enabled(instance: &IdentityInstance) -> bool {
    instance
        .spec
        .agents
        .as_ref()
        .is_some_and(|agents| agents.enabled)
}

pub fn settled_off(revision: u64) -> AgentsStatus {
    AgentsStatus {
        phase: AgentsPhase::Ready,
        gateway_client_ready: false,
        observed_revision: revision,
        mcp_url: None,
        message: None,
        policies: Vec::new(),
    }
}

pub fn authz_required(instance: &IdentityInstance) -> bool {
    if agents_enabled(instance) {
        return true;
    }
    let Some(agents) = instance
        .status
        .as_ref()
        .and_then(|status| status.agents.as_ref())
    else {
        return false;
    };
    *agents != settled_off(agents.observed_revision)
}

pub fn settings_with_resource(current: &RealmSettings, canonical: &str) -> Option<RealmSettings> {
    let present = current
        .allowed_resources
        .iter()
        .any(|resource| resource == canonical);
    if present && current.cimd_enabled {
        return None;
    }

    let mut allowed_resources = current.allowed_resources.clone();
    if !present {
        allowed_resources.push(canonical.to_string());
    }
    Some(RealmSettings {
        allowed_resources,
        cimd_enabled: true,
        // Registration is the realm owner's choice; agents only need CIMD.
        dcr_enabled: current.dcr_enabled,
    })
}

pub fn settings_without_resource(
    current: &RealmSettings,
    canonical: &str,
) -> Option<RealmSettings> {
    if !current
        .allowed_resources
        .iter()
        .any(|resource| resource == canonical)
    {
        return None;
    }

    Some(RealmSettings {
        allowed_resources: current
            .allowed_resources
            .iter()
            .filter(|resource| resource.as_str() != canonical)
            .cloned()
            .collect(),
        ..current.clone()
    })
}

#[cfg(test)]
pub(crate) fn test_spec(
    host: &str,
) -> autharie_crds::v1alpha::identity_instance::IdentityInstanceSpec {
    serde_json::from_value(serde_json::json!({
        "organisationId": "org",
        "provider": "ferriskey",
        "version": "1.0.0",
        "hostname": host,
        "database": { "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} } },
    }))
    .expect("a spec")
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_crds::v1alpha::identity_instance::{
        AgentPolicyPhase, AgentPolicyStatus, AgentsSpec, IdentityInstanceStatus,
    };

    const ID: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const CANONICAL: &str = "https://auth.acme.test/mcp";

    fn instance(enabled: Option<bool>, status: Option<AgentsStatus>) -> IdentityInstance {
        let mut instance = IdentityInstance::new("acme", test_spec("auth.acme.test"));
        instance.spec.agents = enabled.map(|enabled| AgentsSpec {
            enabled,
            realm: "master".to_string(),
            policies: Vec::new(),
            policies_revision: 0,
        });
        instance.status = status.map(|agents| IdentityInstanceStatus {
            agents: Some(agents),
            ..Default::default()
        });
        instance
    }

    #[test]
    fn a_managed_name_carries_the_autharie_id() {
        assert_eq!(managed_name(ID), format!("autharie:{ID}"));
        assert_eq!(managed_id(&managed_name(ID)), Some(ID));
    }

    #[test]
    fn names_that_are_not_ours_are_not_managed() {
        for name in [
            "readers",
            "autharie:",
            "autharie:something",
            "Autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            "x-autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            "autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11-copy",
        ] {
            assert_eq!(managed_id(name), None, "{name}");
        }
    }

    #[test]
    fn the_description_is_the_trimmed_human_name() {
        let spec = AgentPolicySpec {
            id: ID.to_string(),
            name: "  Read only ".to_string(),
            cedar: String::new(),
        };

        assert_eq!(remote_description(&spec), "Read only");
    }

    #[test]
    fn the_exchange_audience_is_the_realm_api_client() {
        assert_eq!(evaluator_audience("acme"), "acme-realm");
    }

    #[test]
    fn authorization_is_required_while_agents_are_on() {
        assert!(authz_required(&instance(Some(true), None)));
    }

    #[test]
    fn authorization_is_not_required_when_agents_never_were() {
        assert!(!authz_required(&instance(None, None)));
        assert!(!authz_required(&instance(Some(false), None)));
    }

    #[test]
    fn authorization_stays_required_until_the_cleanup_has_settled() {
        let pending = AgentsStatus {
            phase: AgentsPhase::Ready,
            gateway_client_ready: true,
            observed_revision: 2,
            mcp_url: Some(CANONICAL.to_string()),
            message: None,
            policies: vec![AgentPolicyStatus {
                id: ID.to_string(),
                phase: AgentPolicyPhase::Applied,
                message: None,
            }],
        };

        assert!(authz_required(&instance(Some(false), Some(pending))));
        assert!(!authz_required(&instance(
            Some(false),
            Some(settled_off(2))
        )));
    }

    #[test]
    fn a_realm_missing_everything_gets_the_resource_cimd_on_and_dcr_off() {
        let wanted = settings_with_resource(&RealmSettings::default(), CANONICAL).expect("change");

        assert_eq!(wanted.allowed_resources, vec![CANONICAL.to_string()]);
        assert!(wanted.cimd_enabled);
        assert!(!wanted.dcr_enabled);
    }

    #[test]
    fn other_allowed_resources_are_kept() {
        let current = RealmSettings {
            allowed_resources: vec!["https://other.test/mcp".to_string()],
            cimd_enabled: true,
            dcr_enabled: true,
        };

        let wanted = settings_with_resource(&current, CANONICAL).expect("change");

        assert_eq!(
            wanted.allowed_resources,
            vec!["https://other.test/mcp".to_string(), CANONICAL.to_string()]
        );
        assert!(
            wanted.dcr_enabled,
            "the owner's registration choice is kept"
        );
    }

    #[test]
    fn a_realm_already_right_is_left_alone() {
        let current = RealmSettings {
            allowed_resources: vec![CANONICAL.to_string()],
            cimd_enabled: true,
            dcr_enabled: false,
        };

        assert_eq!(settings_with_resource(&current, CANONICAL), None);
    }

    #[test]
    fn only_the_canonical_resource_is_taken_back() {
        let current = RealmSettings {
            allowed_resources: vec!["https://other.test/mcp".to_string(), CANONICAL.to_string()],
            cimd_enabled: true,
            dcr_enabled: false,
        };

        let wanted = settings_without_resource(&current, CANONICAL).expect("change");

        assert_eq!(
            wanted.allowed_resources,
            vec!["https://other.test/mcp".to_string()]
        );
        assert!(wanted.cimd_enabled);
        assert_eq!(settings_without_resource(&wanted, CANONICAL), None);
    }

    #[test]
    fn a_gateway_client_never_prints_its_secret() {
        let client = GatewayClient {
            secret: "very-secret".to_string(),
        };

        assert!(!format!("{client:?}").contains("very-secret"));
    }
}
