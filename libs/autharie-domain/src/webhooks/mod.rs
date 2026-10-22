use thiserror::Error;
use uuid::Uuid;

use crate::action::Action;

pub mod commands;
pub mod delivery;
pub mod entities;
mod events;
pub mod ports;
pub mod service;
#[cfg(test)]
mod service_tests;
pub mod value_objects;

pub use delivery::{DeliveryQuery, DeliveryStatus, WebhookDelivery};
pub use entities::{
    RequestKind, RequestPhase, RevealedSecret, SecretRecord, StoredWebhook, Webhook,
    WebhookRequest, WebhookState, WebhookSync, WebhookView,
};
pub use value_objects::{
    HeaderInput, HeaderName, HeaderValue, WebhookEndpoint, WebhookEvent, WebhookEventInfo,
    WebhookId, WebhookName, WebhookRealm, known_events,
};

pub const WEBHOOKS_ACTION_TYPE: &str = "deployment.webhooks";
pub const ROTATE_SECRET_ACTION_TYPE: &str = "deployment.webhook.rotate_secret";
pub const RETRY_DELIVERY_ACTION_TYPE: &str = "deployment.webhook.retry_delivery";

pub const WEBHOOK_CREATED_AUDIT: &str = "deployment.webhook.created";
pub const WEBHOOK_UPDATED_AUDIT: &str = "deployment.webhook.updated";
pub const WEBHOOK_DELETED_AUDIT: &str = "deployment.webhook.deleted";
pub const WEBHOOK_ROTATED_AUDIT: &str = "deployment.webhook.secret_rotated";
pub const WEBHOOK_REVEALED_AUDIT: &str = "deployment.webhook.secret_revealed";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WebhookError {
    #[error("{reason}")]
    Invalid { reason: String },

    #[error("webhook {id} not found")]
    NotFound { id: Uuid },

    #[error("webhooks are only available on FerrisKey deployments")]
    UnsupportedKind,

    #[error("no signing secret has been received for this webhook yet")]
    SecretNotReceived,
}

impl WebhookError {
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }
}

pub fn redacted(mut action: Action) -> Action {
    if action.action_type.0 == WEBHOOKS_ACTION_TYPE
        && let Some(payload) = action.payload.data.as_object_mut()
    {
        payload.remove("headers");
    }

    action
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::{
        action::{
            ActionConstraints, ActionId, ActionMetadata, ActionPayload, ActionSource, ActionStatus,
            ActionTarget, ActionType, ActionVersion, TargetKind,
        },
        dataplane::value_objects::DataPlaneId,
    };

    fn action(action_type: &str) -> Action {
        Action {
            id: ActionId(Uuid::nil()),
            deployment_id: None,
            dataplane_id: DataPlaneId(Uuid::nil()),
            action_type: ActionType(action_type.to_string()),
            target: ActionTarget {
                kind: TargetKind::Deployment,
                id: Uuid::nil(),
            },
            payload: ActionPayload {
                data: json!({ "revision": 3, "headers": { "w": { "x-key": "hunter2" } } }),
            },
            version: ActionVersion(1),
            status: ActionStatus::Pending,
            metadata: ActionMetadata {
                source: ActionSource::System,
                created_at: Utc::now(),
                constraints: ActionConstraints::default(),
            },
            leased_until: None,
        }
    }

    #[test]
    fn redaction_drops_the_header_values_of_a_webhooks_action() {
        let redacted = redacted(action(WEBHOOKS_ACTION_TYPE));

        assert_eq!(redacted.payload.data, json!({ "revision": 3 }));
    }

    #[test]
    fn redaction_leaves_other_actions_alone() {
        let before = action("deployment.iam_settings");

        assert_eq!(redacted(before.clone()), before);
    }
}
