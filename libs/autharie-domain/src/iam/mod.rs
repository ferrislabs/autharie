use chrono::Duration;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::action::Action;

pub mod entities;
pub mod envelope;
pub mod operation;
pub mod payload;
pub mod ports;
pub mod service;
#[cfg(test)]
mod service_tests;

pub use entities::{IamFetch, IamInput, IamOutcome, IamRealm, IamRequest, IamState, IamVerdict};
pub use envelope::{Envelope, MAX_ENVELOPE_BYTES};
pub use operation::{
    IamAccess, IamOperation, IamOperationKind, MAX_PAGE_SIZE, Page, PageSize, SearchTerm,
};
pub use payload::IamActionPayload;

pub const IAM_ACTION_TYPE: &str = "deployment.iam.request";
pub const IAM_REQUESTED_AUDIT: &str = "deployment.iam.requested";
pub const MAX_OUTSTANDING_REQUESTS: usize = 10;

pub const REQUEST_LIFETIME: Duration = Duration::minutes(5);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IamError {
    #[error("{reason}")]
    Invalid { reason: String },

    #[error("the identity channel is only available on FerrisKey deployments")]
    UnsupportedKind,

    #[error("this deployment already has {max} requests waiting: try again in a moment")]
    TooManyRequests { max: usize },

    #[error("no request {request_id} is waiting on this deployment")]
    NotFound { request_id: Uuid },

    #[error("the answer to request {request_id} was already collected")]
    Gone { request_id: Uuid },

    #[error("{message}")]
    Failed { message: String },
}

impl IamError {
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }
}

pub fn redacted(mut action: Action) -> Action {
    if action.action_type.0 == IAM_ACTION_TYPE
        && let Some(payload) = action.payload.data.as_object_mut()
        && let Some(operation) = payload.get_mut("operation")
    {
        let kind = operation.get("type").cloned().unwrap_or(Value::Null);
        *operation = json!({ "type": kind });
    }

    action
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{
        action::{
            ActionConstraints, ActionId, ActionMetadata, ActionPayload, ActionSource, ActionStatus,
            ActionTarget, ActionType, ActionVersion, TargetKind,
        },
        dataplane::value_objects::DataPlaneId,
    };

    fn action(action_type: &str, data: Value) -> Action {
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
    fn redaction_keeps_the_operation_type_and_drops_the_rest() {
        let redacted = redacted(action(
            IAM_ACTION_TYPE,
            json!({
                "realm": "master",
                "operation": { "type": "applications.list", "page": 1, "limit": 25, "search": "alice" }
            }),
        ));

        assert_eq!(
            redacted.payload.data,
            json!({ "realm": "master", "operation": { "type": "applications.list" } })
        );
    }

    #[test]
    fn redaction_leaves_other_actions_alone() {
        let before = action(
            "deployment.smtp",
            json!({ "operation": { "type": "x", "search": "kept" } }),
        );

        assert_eq!(redacted(before.clone()), before);
    }
}
