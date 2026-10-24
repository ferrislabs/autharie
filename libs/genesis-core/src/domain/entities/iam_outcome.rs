use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    #[default]
    Iam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IamOutcomePhase {
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamOutcomeReport {
    pub deployment_id: Uuid,
    #[serde(default)]
    kind: Kind,
    pub request_id: String,
    pub phase: IamOutcomePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Value>,
}

impl IamOutcomeReport {
    pub fn done(deployment_id: Uuid, request_id: impl Into<String>, envelope: Value) -> Self {
        Self {
            deployment_id,
            kind: Kind::Iam,
            request_id: request_id.into(),
            phase: IamOutcomePhase::Done,
            message: None,
            envelope: Some(envelope),
        }
    }

    pub fn failed(
        deployment_id: Uuid,
        request_id: impl Into<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            deployment_id,
            kind: Kind::Iam,
            request_id: request_id.into(),
            phase: IamOutcomePhase::Failed,
            message: Some(message.unwrap_or_else(|| "the request failed".to_string())),
            envelope: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    #[test]
    fn a_done_outcome_carries_the_envelope_and_no_message() {
        let envelope =
            json!({ "v": 1, "alg": "RSA-OAEP-256+A256GCM", "ek": "a", "iv": "b", "ct": "c" });

        let report = IamOutcomeReport::done(DEPLOYMENT, "r1", envelope.clone());

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "iam",
                "request_id": "r1",
                "phase": "Done",
                "envelope": envelope,
            })
        );
    }

    #[test]
    fn a_failed_outcome_carries_its_message_and_no_envelope() {
        let report = IamOutcomeReport::failed(DEPLOYMENT, "r2", Some("result too large".into()));

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "iam",
                "request_id": "r2",
                "phase": "Failed",
                "message": "result too large",
            })
        );
    }

    #[test]
    fn a_failure_without_a_reason_still_says_something() {
        let report = IamOutcomeReport::failed(DEPLOYMENT, "r3", None);

        assert_eq!(report.message.as_deref(), Some("the request failed"));
    }
}
