use thiserror::Error;

use crate::action::Action;

pub mod commands;
pub mod entities;
pub mod payload;
pub mod ports;
pub mod service;
#[cfg(test)]
mod service_tests;
pub mod value_objects;

pub use commands::{RawSmtpInput, SmtpInput, SmtpOutcome};
pub use entities::{SmtpConfig, SmtpSettings, SmtpState, SmtpView, StoredSmtp};
pub use payload::{SmtpActionPayload, SmtpActionSettings};
pub use value_objects::{
    FromEmail, FromName, SmtpEncryption, SmtpHost, SmtpPassword, SmtpPort, SmtpRealm, SmtpUsername,
};

pub const SMTP_ACTION_TYPE: &str = "deployment.smtp";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SmtpError {
    #[error("{reason}")]
    Invalid { reason: String },

    #[error("a password is required the first time the email provider is saved")]
    PasswordRequired,

    #[error("no email provider is set for the realm '{realm}'")]
    NotFound { realm: String },

    #[error("the email provider is only available on FerrisKey deployments")]
    UnsupportedKind,
}

impl SmtpError {
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }
}

pub fn redacted(mut action: Action) -> Action {
    if action.action_type.0 == SMTP_ACTION_TYPE
        && let Some(smtp) = action
            .payload
            .data
            .get_mut("smtp")
            .and_then(|smtp| smtp.as_object_mut())
    {
        smtp.remove("password");
    }

    action
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::{
        action::{
            ActionConstraints, ActionId, ActionMetadata, ActionPayload, ActionSource, ActionStatus,
            ActionTarget, ActionType, ActionVersion, TargetKind,
        },
        dataplane::value_objects::DataPlaneId,
    };

    fn action(action_type: &str, data: serde_json::Value) -> Action {
        Action {
            id: ActionId(Uuid::nil()),
            deployment_id: None,
            dataplane_id: DataPlaneId(Uuid::nil()),
            action_type: ActionType(action_type.to_string()),
            target: ActionTarget {
                kind: TargetKind::Deployment,
                id: Uuid::nil(),
            },
            payload: ActionPayload { data },
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
    fn redaction_drops_the_password_of_an_smtp_action() {
        let redacted = redacted(action(
            SMTP_ACTION_TYPE,
            json!({ "revision": 3, "smtp": { "host": "mail.example.com", "password": "hunter2" } }),
        ));

        assert_eq!(
            redacted.payload.data,
            json!({ "revision": 3, "smtp": { "host": "mail.example.com" } })
        );
    }

    #[test]
    fn redaction_leaves_a_removal_alone() {
        let before = action(SMTP_ACTION_TYPE, json!({ "revision": 3, "smtp": null }));

        assert_eq!(redacted(before.clone()), before);
    }

    #[test]
    fn redaction_leaves_other_actions_alone() {
        let before = action(
            "deployment.webhooks",
            json!({ "smtp": { "password": "kept" } }),
        );

        assert_eq!(redacted(before.clone()), before);
    }
}
