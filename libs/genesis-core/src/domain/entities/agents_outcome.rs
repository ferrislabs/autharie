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
        }
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
