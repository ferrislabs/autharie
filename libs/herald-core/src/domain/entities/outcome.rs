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

pub const WEBHOOKS_KIND: &str = "webhooks";

#[derive(Clone, PartialEq, Eq)]
pub struct WebhooksOutcome {
    deployment_id: Uuid,
    body: serde_json::Value,
}

impl WebhooksOutcome {
    pub fn deployment_id(&self) -> Uuid {
        self.deployment_id
    }

    pub fn body(&self) -> &serde_json::Value {
        &self.body
    }

    pub fn counts(&self) -> WebhooksOutcomeCounts {
        let len = |key: &str| {
            self.body
                .get(key)
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len)
        };

        WebhooksOutcomeCounts {
            webhooks: len("webhooks"),
            secrets: len("secrets"),
            realms: len("realms"),
            requests: len("requests"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebhooksOutcomeCounts {
    pub webhooks: usize,
    pub secrets: usize,
    pub realms: usize,
    pub requests: usize,
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

/// Features whose outcome Herald only carries: it reads the kind and the
/// deployment, and hands the body to the control plane whole.
pub const RELAYED_KINDS: [&str; 3] = ["smtp", "portal_theme", "iam"];

#[derive(Clone, PartialEq, Eq)]
pub struct RelayedOutcome {
    deployment_id: Uuid,
    kind: String,
    body: serde_json::Value,
}

impl RelayedOutcome {
    pub fn deployment_id(&self) -> Uuid {
        self.deployment_id
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn body(&self) -> &serde_json::Value {
        &self.body
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum InboundOutcome {
    Report(DeploymentOutcomeReport),
    Webhooks(WebhooksOutcome),
    Agents(AgentsOutcome),
    Relayed(RelayedOutcome),
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
            Self::Webhooks(outcome) => outcome.deployment_id,
            Self::Agents(outcome) => outcome.deployment_id,
            Self::Relayed(outcome) => outcome.deployment_id,
        }
    }

    pub fn parse(payload: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|err| format!("not JSON (line {}, column {})", err.line(), err.column()))?;

        let kind = value.get("kind").and_then(serde_json::Value::as_str);
        let is_webhooks = kind == Some(WEBHOOKS_KIND);
        let is_agents = kind == Some(AGENTS_KIND);
        let relayed = kind.filter(|kind| RELAYED_KINDS.contains(kind));

        if !is_webhooks && !is_agents && relayed.is_none() {
            return serde_json::from_value(value)
                .map(Self::Report)
                .map_err(|err| err.to_string());
        }

        let deployment_id = value
            .get("deployment_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| {
                if is_webhooks {
                    "a webhooks outcome without a valid deployment_id".to_string()
                } else if is_agents {
                    "an agents outcome without a valid deployment_id".to_string()
                } else {
                    format!(
                        "a {} outcome without a valid deployment_id",
                        relayed.unwrap_or_default()
                    )
                }
            })?;

        if let Some(kind) = relayed {
            return Ok(Self::Relayed(RelayedOutcome {
                deployment_id,
                kind: kind.to_string(),
                body: value,
            }));
        }

        if is_webhooks {
            Ok(Self::Webhooks(WebhooksOutcome {
                deployment_id,
                body: value,
            }))
        } else {
            Ok(Self::Agents(AgentsOutcome {
                deployment_id,
                body: value,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: &str = "44444444-4444-4444-4444-444444444444";

    fn webhooks_payload() -> serde_json::Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "kind": "webhooks",
            "revision": 3,
            "webhooks": [{ "id": "w1", "phase": "Active", "secret_version": 1 }],
            "secrets": [{ "id": "w1", "version": 1, "secret": "whsec_very_secret" }],
            "realms": ["master", "acme"],
            "requests": []
        })
    }

    #[test]
    fn a_webhooks_outcome_is_kept_whole() {
        let payload = webhooks_payload();

        let parsed = InboundOutcome::parse(payload.to_string().as_bytes()).expect("parses");

        let InboundOutcome::Webhooks(outcome) = parsed else {
            panic!("expected a webhooks outcome");
        };
        assert_eq!(
            outcome.deployment_id(),
            Uuid::parse_str(DEPLOYMENT).unwrap()
        );
        assert_eq!(outcome.body(), &payload);
        assert_eq!(
            outcome.counts(),
            WebhooksOutcomeCounts {
                webhooks: 1,
                secrets: 1,
                realms: 2,
                requests: 0
            }
        );
    }

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
    fn a_webhooks_outcome_without_a_deployment_is_refused_without_quoting_it() {
        let payload = json!({
            "deployment_id": "not-a-uuid",
            "kind": "webhooks",
            "secrets": [{ "id": "w1", "version": 1, "secret": "whsec_very_secret" }]
        });

        let Err(message) = InboundOutcome::parse(payload.to_string().as_bytes()) else {
            panic!("expected a refusal");
        };

        assert!(!message.contains("whsec_very_secret"));
        assert!(!message.contains("not-a-uuid"));
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

    #[test]
    fn an_smtp_portal_theme_or_iam_outcome_is_kept_whole() {
        for kind in RELAYED_KINDS {
            let payload = json!({
                "deployment_id": DEPLOYMENT,
                "kind": kind,
                "phase": "Done",
                "request_id": "55555555-5555-5555-5555-555555555555",
            });

            let parsed = InboundOutcome::parse(payload.to_string().as_bytes()).expect("parses");

            let InboundOutcome::Relayed(outcome) = parsed else {
                panic!("expected a relayed outcome for {kind}");
            };
            assert_eq!(outcome.kind(), kind);
            assert_eq!(
                outcome.deployment_id(),
                Uuid::parse_str(DEPLOYMENT).unwrap()
            );
            assert_eq!(outcome.body(), &payload);
        }
    }

    #[test]
    fn a_relayed_outcome_without_a_valid_deployment_is_refused_by_name() {
        let payload = json!({ "kind": "iam", "phase": "Done" });

        let error = InboundOutcome::parse(payload.to_string().as_bytes())
            .err()
            .expect("refused");

        assert!(error.contains("iam outcome"), "{error}");
    }

    #[test]
    fn a_kind_nobody_relays_is_still_read_as_a_lifecycle_report() {
        let payload =
            json!({ "deployment_id": DEPLOYMENT, "kind": "unknown", "outcome": "running" });

        let parsed = InboundOutcome::parse(payload.to_string().as_bytes()).expect("parses");

        assert!(matches!(parsed, InboundOutcome::Report(_)));
    }
}
