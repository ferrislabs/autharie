use std::fmt;

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    dataplane::{credential::SecretString, credential_repository::SealedSecret},
    deployments::DeploymentId,
    webhooks::{
        WebhookError,
        value_objects::{
            HeaderName, WebhookEndpoint, WebhookEvent, WebhookId, WebhookName, WebhookRealm,
        },
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "phase")]
pub enum WebhookState {
    Pending,
    Active { ferriskey_id: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Webhook {
    pub id: WebhookId,
    pub deployment_id: DeploymentId,
    pub realm: WebhookRealm,
    pub name: WebhookName,
    pub description: Option<String>,
    pub endpoint: WebhookEndpoint,
    pub events: Vec<WebhookEvent>,
    pub headers: Vec<HeaderName>,
    pub state: WebhookState,
    pub secret_version: Option<u64>,
    pub secret_received_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub desired_revision: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWebhook {
    pub webhook: Webhook,
    pub sealed_headers: Option<SealedSecret>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRecord {
    pub version: u64,
    pub sealed: SealedSecret,
    pub received_at: DateTime<Utc>,
}

pub struct RevealedSecret {
    pub secret: SecretString,
    pub version: u64,
}

impl fmt::Debug for RevealedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevealedSecret")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    RotateSecret,
    RetryDelivery,
}

impl RequestKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RotateSecret => "rotate_secret",
            Self::RetryDelivery => "retry_delivery",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "rotate_secret" => Some(Self::RotateSecret),
            "retry_delivery" => Some(Self::RetryDelivery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestPhase {
    Pending,
    Done,
    Failed,
}

impl RequestPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn from_report(value: &str) -> Result<Self, WebhookError> {
        match value {
            "Pending" => Ok(Self::Pending),
            "Done" => Ok(Self::Done),
            "Failed" => Ok(Self::Failed),
            other => Err(WebhookError::invalid(format!(
                "'{other}' is not a request phase"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct WebhookRequest {
    pub id: Uuid,
    pub webhook_id: WebhookId,
    pub kind: RequestKind,
    pub delivery_id: Option<String>,
    pub phase: RequestPhase,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct WebhookView {
    #[serde(flatten)]
    pub webhook: Webhook,
    pub requests: Vec<WebhookRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebhookSync {
    pub revision: u64,
    pub applied_revision: u64,
    pub realms: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_phase_reads_the_wire_names() {
        assert_eq!(RequestPhase::from_report("Done"), Ok(RequestPhase::Done));
        assert_eq!(
            RequestPhase::from_report("Failed"),
            Ok(RequestPhase::Failed)
        );
        assert!(RequestPhase::from_report("done").is_err());
    }

    #[test]
    fn a_state_serialises_with_its_phase() {
        let value = serde_json::to_value(WebhookState::Active {
            ferriskey_id: "fk".to_string(),
        })
        .unwrap();

        assert_eq!(
            value,
            serde_json::json!({ "phase": "Active", "ferriskey_id": "fk" })
        );
    }

    #[test]
    fn a_revealed_secret_does_not_print() {
        let revealed = RevealedSecret {
            secret: SecretString::new("whsec_hunter2"),
            version: 1,
        };

        assert!(!format!("{revealed:?}").contains("hunter2"));
    }
}
