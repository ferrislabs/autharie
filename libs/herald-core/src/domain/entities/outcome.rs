use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// What a data plane component observed happening to a deployment.
///
/// Herald does not produce these -- Genesis does, and Herald carries them to
/// the control plane. That split is deliberate: Genesis is the component that
/// touches Kubernetes and therefore knows, and Herald is the component that
/// holds control plane credentials. Neither gains what the other has.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub administrator: Option<RelayedAdministrator>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct RelayedAdministrator {
    username: String,
    password: String,
}

impl DeploymentOutcomeReport {
    pub fn observed(deployment_id: Uuid, outcome: &str, version: Option<String>) -> Self {
        Self {
            deployment_id,
            outcome: outcome.to_string(),
            version,
            duration_seconds: None,
            reason: None,
            administrator: None,
            request_id: None,
        }
    }
}

impl fmt::Debug for DeploymentOutcomeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeploymentOutcomeReport")
            .field("deployment_id", &self.deployment_id)
            .field("outcome", &self.outcome)
            .field("version", &self.version)
            .field("duration_seconds", &self.duration_seconds)
            .field("reason", &self.reason)
            .field(
                "administrator",
                &self.administrator.as_ref().map(|_| "<redacted>"),
            )
            .field("request_id", &self.request_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "Zq8-temporary-Pass!";

    fn carried() -> DeploymentOutcomeReport {
        serde_json::from_value(serde_json::json!({
            "deployment_id": Uuid::nil(),
            "outcome": "running",
            "administrator": { "username": "admin", "password": PASSWORD },
        }))
        .expect("a report")
    }

    #[test]
    fn an_administrator_is_carried_through_untouched() {
        let forwarded = serde_json::to_value(carried()).expect("serialised");

        assert_eq!(forwarded["administrator"]["username"], "admin");
        assert_eq!(forwarded["administrator"]["password"], PASSWORD);
    }

    #[test]
    fn a_report_never_shows_the_administrator_password() {
        let shown = format!("{:?}", carried());

        assert!(!shown.contains(PASSWORD), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn a_report_without_an_administrator_forwards_no_administrator_key() {
        let report: DeploymentOutcomeReport = serde_json::from_value(serde_json::json!({
            "deployment_id": Uuid::nil(),
            "outcome": "deleted",
        }))
        .expect("a report");

        let forwarded = serde_json::to_value(report).expect("serialised");
        assert!(forwarded.get("administrator").is_none());
    }
}
