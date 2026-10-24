use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::{
    IamError,
    envelope::Envelope,
    operation::{IamOperation, IamOperationKind},
};
use crate::{
    CoreError,
    agents::registry::{SealingJwk, SealingKey},
    deployments::DeploymentId,
    webhooks::WebhookRealm,
};

pub const MAX_MESSAGE_CHARS: usize = 500;

pub type IamRealm = WebhookRealm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamState {
    Pending,
    Done,
    Failed { message: String },
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IamRequest {
    pub id: Uuid,
    pub deployment_id: DeploymentId,
    pub realm: IamRealm,
    pub operation: IamOperationKind,
    pub actor: String,
    pub state: IamState,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IamInput {
    pub realm: IamRealm,
    pub operation: IamOperation,
    pub sealing_key: SealingKey,
}

impl IamInput {
    pub fn parse(
        realm: &str,
        operation: IamOperation,
        sealing_key: SealingJwk,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            realm: IamRealm::parse(realm).map_err(|error| IamError::invalid(error.to_string()))?,
            operation,
            sealing_key: SealingKey::try_from(sealing_key)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamFetch {
    Pending,
    Ready(Envelope),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamVerdict {
    Done(Envelope),
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IamOutcome {
    pub request_id: Uuid,
    pub verdict: IamVerdict,
}

impl IamOutcome {
    pub fn parse(
        request_id: Option<Uuid>,
        phase: Option<&str>,
        message: Option<String>,
        envelope: Option<Value>,
    ) -> Result<Self, IamError> {
        let request_id =
            request_id.ok_or_else(|| IamError::invalid("an iam report needs its request id"))?;

        let verdict = match phase {
            Some("Done") => IamVerdict::Done(Envelope::parse(envelope.ok_or_else(|| {
                IamError::invalid("a finished iam report carries its envelope")
            })?)?),
            Some("Failed") => IamVerdict::Failed {
                message: message
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map_or_else(
                        || "the data plane gave no reason".to_string(),
                        |text| text.chars().take(MAX_MESSAGE_CHARS).collect(),
                    ),
            },
            Some(other) => {
                return Err(IamError::invalid(format!(
                    "'{other}' is not an iam phase: use Done or Failed"
                )));
            }
            None => return Err(IamError::invalid("an iam report needs a phase")),
        };

        Ok(Self {
            request_id,
            verdict,
        })
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    use super::*;

    fn envelope() -> Value {
        json!({
            "v": 1,
            "alg": "RSA-OAEP-256+A256GCM",
            "ek": URL_SAFE_NO_PAD.encode([1u8; 256]),
            "iv": URL_SAFE_NO_PAD.encode([2u8; 12]),
            "ct": URL_SAFE_NO_PAD.encode([3u8; 40]),
        })
    }

    #[test]
    fn a_done_report_carries_its_envelope() {
        let id = Uuid::new_v4();
        let outcome = IamOutcome::parse(Some(id), Some("Done"), None, Some(envelope())).unwrap();

        assert_eq!(outcome.request_id, id);
        assert!(matches!(outcome.verdict, IamVerdict::Done(_)));
    }

    #[test]
    fn a_failed_report_keeps_its_message_cut_to_500_characters_and_never_an_envelope() {
        let outcome = IamOutcome::parse(
            Some(Uuid::new_v4()),
            Some("Failed"),
            Some("x".repeat(900)),
            Some(envelope()),
        )
        .unwrap();

        assert_eq!(
            outcome.verdict,
            IamVerdict::Failed {
                message: "x".repeat(500)
            }
        );
    }

    #[test]
    fn a_failed_report_without_a_reason_says_so() {
        let outcome = IamOutcome::parse(Some(Uuid::new_v4()), Some("Failed"), None, None).unwrap();

        assert_eq!(
            outcome.verdict,
            IamVerdict::Failed {
                message: "the data plane gave no reason".to_string()
            }
        );
    }

    #[test]
    fn a_report_needs_an_id_a_known_phase_and_for_done_a_valid_envelope() {
        let id = Some(Uuid::new_v4());

        assert!(IamOutcome::parse(None, Some("Failed"), None, None).is_err());
        assert!(IamOutcome::parse(id, None, None, None).is_err());
        assert!(IamOutcome::parse(id, Some("Pending"), None, None).is_err());
        assert!(IamOutcome::parse(id, Some("Done"), None, None).is_err());
        assert!(IamOutcome::parse(id, Some("Done"), None, Some(json!({ "ct": "x" }))).is_err());
    }
}
