use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a data plane component observed happening to a deployment.
///
/// Herald does not produce these -- Genesis does, and Herald carries them to
/// the control plane. That split is deliberate: Genesis is the component that
/// touches Kubernetes and therefore knows, and Herald is the component that
/// holds control plane credentials. Neither gains what the other has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentOutcomeReport {
    pub deployment_id: Uuid,
    pub outcome: String,

    /// The version the data plane observed running. Carried through untouched:
    /// Herald does not look at Kubernetes and has no business interpreting it.
    #[serde(default)]
    pub version: Option<String>,

    /// How long a drill (#185) took. Carried through untouched, same as
    /// `version`.
    #[serde(default)]
    pub duration_seconds: Option<u64>,

    /// Why a drill failed. Carried through untouched, same as `version`.
    #[serde(default)]
    pub reason: Option<String>,
}

impl DeploymentOutcomeReport {
    pub fn observed(deployment_id: Uuid, outcome: &str, version: Option<String>) -> Self {
        Self {
            deployment_id,
            outcome: outcome.to_string(),
            version,
            duration_seconds: None,
            reason: None,
        }
    }
}

pub const AGENTS_KIND: &str = "deployment.agents";

#[derive(Clone, PartialEq, Eq)]
pub struct AgentsOutcome {
    deployment_id: Uuid,
    body: serde_json::Value,
}

impl AgentsOutcome {
    pub fn deployment_id(&self) -> Uuid {
        self.deployment_id
    }

    pub fn body(&self) -> &serde_json::Value {
        &self.body
    }

    pub fn policy_count(&self) -> usize {
        self.body
            .get("policies")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum InboundOutcome {
    Report(DeploymentOutcomeReport),
    Agents(AgentsOutcome),
}

impl From<DeploymentOutcomeReport> for InboundOutcome {
    fn from(report: DeploymentOutcomeReport) -> Self {
        Self::Report(report)
    }
}

impl InboundOutcome {
    pub fn deployment_id(&self) -> Uuid {
        match self {
            Self::Report(report) => report.deployment_id,
            Self::Agents(outcome) => outcome.deployment_id,
        }
    }

    pub fn parse(payload: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|err| format!("not JSON (line {}, column {})", err.line(), err.column()))?;

        let is_agents = value.get("kind").and_then(serde_json::Value::as_str) == Some(AGENTS_KIND);

        if !is_agents {
            return serde_json::from_value(value)
                .map(Self::Report)
                .map_err(|err| err.to_string());
        }

        let deployment_id = value
            .get("deployment_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| "an agents outcome without a valid deployment_id".to_string())?;

        Ok(Self::Agents(AgentsOutcome {
            deployment_id,
            body: value,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: &str = "44444444-4444-4444-4444-444444444444";

    fn agents_payload() -> serde_json::Value {
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
            ]
        })
    }

    #[test]
    fn an_agents_outcome_is_kept_whole() {
        let payload = agents_payload();

        let parsed = InboundOutcome::parse(payload.to_string().as_bytes()).expect("parses");

        let InboundOutcome::Agents(outcome) = parsed else {
            panic!("expected an agents outcome");
        };
        assert_eq!(
            outcome.deployment_id(),
            Uuid::parse_str(DEPLOYMENT).unwrap()
        );
        assert_eq!(outcome.body(), &payload);
        assert_eq!(outcome.policy_count(), 2);
    }

    #[test]
    fn a_report_without_a_kind_is_the_lifecycle_report() {
        let payload =
            json!({ "deployment_id": DEPLOYMENT, "outcome": "running", "version": "0.9.0" });

        let parsed = InboundOutcome::parse(payload.to_string().as_bytes()).expect("parses");

        assert!(matches!(
            parsed,
            InboundOutcome::Report(report) if report.outcome == "running"
                && report.version.as_deref() == Some("0.9.0")
        ));
    }

    #[test]
    fn an_agents_outcome_without_a_deployment_is_refused_without_quoting_it() {
        let payload = json!({
            "deployment_id": "not-a-uuid",
            "kind": "deployment.agents",
            "message": "permit (principal == User::\"secret-user\");"
        });

        let Err(message) = InboundOutcome::parse(payload.to_string().as_bytes()) else {
            panic!("expected a refusal");
        };

        assert!(!message.contains("secret-user"));
        assert!(!message.contains("not-a-uuid"));
    }

    #[test]
    fn a_message_that_is_not_json_is_refused() {
        assert!(InboundOutcome::parse(b"{nope").is_err());
    }
}
