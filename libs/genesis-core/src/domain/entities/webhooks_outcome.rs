use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    #[default]
    Webhooks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookOutcome {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ferriskey_id: Option<String>,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub secret_version: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretOutcome {
    pub id: String,
    pub version: u64,
    pub secret: String,
}

impl fmt::Debug for SecretOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretOutcome")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestOutcome {
    pub id: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhooksOutcomeReport {
    pub deployment_id: Uuid,
    #[serde(default)]
    kind: Kind,
    pub revision: u64,
    pub webhooks: Vec<WebhookOutcome>,
    pub secrets: Vec<SecretOutcome>,
    pub realms: Vec<String>,
    pub requests: Vec<RequestOutcome>,
}

impl WebhooksOutcomeReport {
    pub fn new(
        deployment_id: Uuid,
        revision: u64,
        webhooks: Vec<WebhookOutcome>,
        secrets: Vec<SecretOutcome>,
        realms: Vec<String>,
        requests: Vec<RequestOutcome>,
    ) -> Self {
        Self {
            deployment_id,
            kind: Kind::Webhooks,
            revision,
            webhooks,
            secrets,
            realms,
            requests,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::outcome::DeploymentOutcomeReport;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    #[test]
    fn it_serialises_in_the_frozen_shape() {
        let report = WebhooksOutcomeReport::new(
            DEPLOYMENT,
            3,
            vec![WebhookOutcome {
                id: "w1".to_string(),
                ferriskey_id: Some("f1".to_string()),
                phase: "Active".to_string(),
                message: None,
                secret_version: 2,
            }],
            vec![SecretOutcome {
                id: "w1".to_string(),
                version: 2,
                secret: "shh".to_string(),
            }],
            vec!["master".to_string()],
            vec![RequestOutcome {
                id: "r1".to_string(),
                phase: "Failed".to_string(),
                message: Some("nope".to_string()),
            }],
        );

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "webhooks",
                "revision": 3,
                "webhooks": [
                    { "id": "w1", "ferriskey_id": "f1", "phase": "Active", "secret_version": 2 }
                ],
                "secrets": [{ "id": "w1", "version": 2, "secret": "shh" }],
                "realms": ["master"],
                "requests": [{ "id": "r1", "phase": "Failed", "message": "nope" }],
            })
        );
    }

    #[test]
    fn a_secret_is_never_printed_by_debug() {
        let secret = SecretOutcome {
            id: "w1".to_string(),
            version: 1,
            secret: "shh-do-not-print".to_string(),
        };

        assert!(!format!("{secret:?}").contains("shh-do-not-print"));
    }

    #[test]
    fn the_older_report_has_no_kind() {
        let value =
            serde_json::to_value(DeploymentOutcomeReport::deleted(DEPLOYMENT)).expect("serialises");

        assert!(value.get("kind").is_none());
    }
}
