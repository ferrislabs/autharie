use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const AGENTS_KIND: &str = "deployment.agents";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPolicyOutcome {
    pub id: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentClientReport {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: String,
    pub kind: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCredentialReport {
    pub request_id: String,
    pub client_id: String,
    pub sealed: String,
}

impl std::fmt::Debug for AgentCredentialReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentCredentialReport")
            .field("request_id", &self.request_id)
            .field("client_id", &self.client_id)
            .field("sealed", &"<sealed>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsOutcomeReport {
    pub deployment_id: Uuid,
    kind: String,
    pub revision: u64,
    pub phase: String,
    pub gateway_client_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub policies: Vec<AgentPolicyOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clients: Vec<AgentClientReport>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clients_truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<AgentCredentialReport>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl AgentsOutcomeReport {
    pub fn new(
        deployment_id: Uuid,
        revision: u64,
        phase: impl Into<String>,
        gateway_client_ready: bool,
        mcp_url: Option<String>,
        message: Option<String>,
        policies: Vec<AgentPolicyOutcome>,
    ) -> Self {
        Self {
            deployment_id,
            kind: AGENTS_KIND.to_string(),
            revision,
            phase: phase.into(),
            gateway_client_ready,
            mcp_url,
            message,
            policies,
            clients: Vec::new(),
            clients_truncated: false,
            credentials: Vec::new(),
        }
    }

    pub fn with_registry(
        mut self,
        clients: Vec<AgentClientReport>,
        clients_truncated: bool,
        credentials: Vec<AgentCredentialReport>,
    ) -> Self {
        self.clients = clients;
        self.clients_truncated = clients_truncated;
        self.credentials = credentials;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    #[test]
    fn it_serialises_in_the_frozen_shape() {
        let report = AgentsOutcomeReport::new(
            DEPLOYMENT,
            3,
            "Ready",
            true,
            Some("https://auth.example.com/mcp".to_string()),
            None,
            vec![
                AgentPolicyOutcome {
                    id: "p1".to_string(),
                    phase: "Applied".to_string(),
                    message: None,
                },
                AgentPolicyOutcome {
                    id: "p2".to_string(),
                    phase: "Invalid".to_string(),
                    message: Some("unexpected token".to_string()),
                },
            ],
        );

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "deployment.agents",
                "revision": 3,
                "phase": "Ready",
                "gateway_client_ready": true,
                "mcp_url": "https://auth.example.com/mcp",
                "policies": [
                    { "id": "p1", "phase": "Applied" },
                    { "id": "p2", "phase": "Invalid", "message": "unexpected token" }
                ],
            })
        );
    }

    #[test]
    fn the_registry_is_reported_in_the_frozen_shape() {
        let report = AgentsOutcomeReport::new(DEPLOYMENT, 3, "Ready", true, None, None, vec![])
            .with_registry(
                vec![AgentClientReport {
                    client_id: "dcr".to_string(),
                    name: None,
                    source: "dynamic".to_string(),
                    kind: "delegated".to_string(),
                    enabled: true,
                    created_at: Some("2026-10-23T10:00:00Z".to_string()),
                }],
                true,
                vec![AgentCredentialReport {
                    request_id: "r1".to_string(),
                    client_id: "agent-x-abc123".to_string(),
                    sealed: "c2VhbGVk".to_string(),
                }],
            );

        let value = serde_json::to_value(&report).expect("serialises");

        assert_eq!(
            value["clients"],
            json!([{ "client_id": "dcr", "source": "dynamic", "kind": "delegated",
                     "enabled": true, "created_at": "2026-10-23T10:00:00Z" }])
        );
        assert_eq!(value["clients_truncated"], true);
        assert_eq!(
            value["credentials"],
            json!([{ "request_id": "r1", "client_id": "agent-x-abc123", "sealed": "c2VhbGVk" }])
        );
        assert!(!format!("{report:?}").contains("c2VhbGVk"));
    }

    #[test]
    fn a_failed_report_carries_its_message_and_no_url() {
        let report = AgentsOutcomeReport::new(
            DEPLOYMENT,
            1,
            "Failed",
            false,
            None,
            Some("FerrisKey is unreachable".to_string()),
            Vec::new(),
        );

        let value = serde_json::to_value(&report).expect("serialises");

        assert_eq!(value["message"], "FerrisKey is unreachable");
        assert!(value.get("mcp_url").is_none());
        assert_eq!(value["policies"], json!([]));
    }
}
