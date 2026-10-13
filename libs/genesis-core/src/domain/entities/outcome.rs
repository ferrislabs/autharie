use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::tenant_access::InitialAdministrator;

/// What Genesis observed happening to a deployment, on its way back to the
/// control plane.
///
/// Carries no data plane id on purpose. Herald addresses the report to the data
/// plane it is configured for, which is the one identity in this path that is
/// authenticated -- a data plane id travelling in the payload would be an
/// unverified claim sitting next to a verified one, and sooner or later
/// something would trust the wrong one.
///
/// Published to the broker rather than sent to the control plane directly, and
/// that is the point: Genesis holds AMQP credentials and Herald holds control
/// plane credentials, and this keeps it that way. Reporting directly would
/// mean a second component learning how to authenticate against the control
/// plane, and duplicating the token refresh that already lives in Herald.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentOutcomeReport {
    pub deployment_id: Uuid,
    /// The vocabulary the control plane parses. A string rather than an enum
    /// because it crosses a process boundary: an outcome this Genesis does not
    /// know about must not stop an older one deserialising the ones it does.
    pub outcome: String,

    /// The version observed running, when the observation says one.
    ///
    /// Skipped when absent so a control plane built before this field reads
    /// the report unchanged. Adding a field is safe in that direction; the
    /// reverse is not, which is why the consumer went first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// How long a drill (#185) took, start to finish. Absent for every
    /// outcome that is not a drill result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,

    /// Why a drill failed, for the audit log a failed drill writes. Absent
    /// for every outcome that is not a failed drill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub administrator: Option<InitialAdministrator>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
}

impl fmt::Debug for DeploymentOutcomeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let administrator = self.administrator.as_ref().map(|_| "<redacted>");
        f.debug_struct("DeploymentOutcomeReport")
            .field("deployment_id", &self.deployment_id)
            .field("outcome", &self.outcome)
            .field("version", &self.version)
            .field("duration_seconds", &self.duration_seconds)
            .field("reason", &self.reason)
            .field("administrator", &administrator)
            .field("request_id", &self.request_id)
            .finish()
    }
}

impl DeploymentOutcomeReport {
    /// The resources are gone.
    ///
    /// Reported by the component that removed them, which is what makes it the
    /// one outcome a data plane can state without inferring anything.
    pub fn deleted(deployment_id: Uuid) -> Self {
        Self {
            deployment_id,
            outcome: "deleted".to_string(),
            version: None,
            duration_seconds: None,
            reason: None,
            administrator: None,
            request_id: None,
        }
    }

    /// The instance is serving, on this version.
    ///
    /// The version is what tells an upgrade that landed from one that has not
    /// started: an instance answers on its old version until the rollout
    /// replaces it, and this watcher resyncs long before that.
    pub fn running(deployment_id: Uuid, version: impl Into<String>) -> Self {
        Self {
            deployment_id,
            outcome: "running".to_string(),
            version: Some(version.into()),
            duration_seconds: None,
            reason: None,
            administrator: None,
            request_id: None,
        }
    }

    pub fn serving(deployment_id: Uuid) -> Self {
        Self {
            deployment_id,
            outcome: "running".to_string(),
            version: None,
            duration_seconds: None,
            reason: None,
            administrator: None,
            request_id: None,
        }
    }

    pub fn administrator_issued(
        deployment_id: Uuid,
        request_id: Uuid,
        administrator: InitialAdministrator,
    ) -> Self {
        Self {
            deployment_id,
            outcome: "administrator_issued".to_string(),
            version: None,
            duration_seconds: None,
            reason: None,
            administrator: Some(administrator),
            request_id: Some(request_id),
        }
    }

    pub fn failed_because(deployment_id: Uuid, reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::failed(deployment_id)
        }
    }

    pub fn failed(deployment_id: Uuid) -> Self {
        Self {
            deployment_id,
            outcome: "failed".to_string(),
            version: None,
            duration_seconds: None,
            reason: None,
            administrator: None,
            request_id: None,
        }
    }

    /// A drill (#185) restored this deployment's own archive end to end and
    /// confirmed the database answers a query. The duration is the restore
    /// time objective measured rather than claimed.
    pub fn drill_succeeded(deployment_id: Uuid, duration_seconds: u64) -> Self {
        Self {
            deployment_id,
            outcome: "drill_succeeded".to_string(),
            version: None,
            duration_seconds: Some(duration_seconds),
            reason: None,
            administrator: None,
            request_id: None,
        }
    }

    /// A drill (#185) failed. The duration is carried when the failure
    /// happened after the drill started timing, and omitted when it failed
    /// before there was anything to time.
    pub fn drill_failed(
        deployment_id: Uuid,
        reason: impl Into<String>,
        duration_seconds: Option<u64>,
    ) -> Self {
        Self {
            deployment_id,
            outcome: "drill_failed".to_string(),
            version: None,
            duration_seconds,
            reason: Some(reason.into()),
            administrator: None,
            request_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    #[test]
    fn a_succeeded_drill_carries_its_duration_and_no_reason() {
        let report = DeploymentOutcomeReport::drill_succeeded(DEPLOYMENT, 212);

        assert_eq!(report.outcome, "drill_succeeded");
        assert_eq!(report.duration_seconds, Some(212));
        assert_eq!(report.reason, None);
        assert_eq!(report.version, None);
    }

    #[test]
    fn a_failed_drill_carries_its_reason_and_an_optional_duration() {
        let report =
            DeploymentOutcomeReport::drill_failed(DEPLOYMENT, "corrupted archive", Some(64));

        assert_eq!(report.outcome, "drill_failed");
        assert_eq!(report.reason.as_deref(), Some("corrupted archive"));
        assert_eq!(report.duration_seconds, Some(64));
    }

    /// A drill that failed before there was anything to time -- an instance
    /// that never even applied -- still reports, just with nothing to say
    /// about duration.
    #[test]
    fn a_failed_drill_may_carry_no_duration() {
        let report = DeploymentOutcomeReport::drill_failed(DEPLOYMENT, "could not apply", None);

        assert_eq!(report.duration_seconds, None);
    }

    /// Every existing outcome stays untouched by the two new fields: a
    /// control plane reading a `running`/`failed`/`deleted` report sees
    /// exactly what it always has, once the optional fields are skipped.
    #[test]
    fn ordinary_outcomes_carry_neither_new_field() {
        for report in [
            DeploymentOutcomeReport::deleted(DEPLOYMENT),
            DeploymentOutcomeReport::running(DEPLOYMENT, "26.0.1"),
            DeploymentOutcomeReport::failed(DEPLOYMENT),
        ] {
            assert_eq!(report.duration_seconds, None);
            assert_eq!(report.reason, None);
        }
    }

    #[test]
    fn debug_never_shows_the_password() {
        let report = DeploymentOutcomeReport::administrator_issued(
            DEPLOYMENT,
            Uuid::from_u128(9),
            InitialAdministrator::new("admin", "hunter2-Secret"),
        );

        let shown = format!("{report:?}");

        assert!(!shown.contains("hunter2-Secret"), "{shown}");
        assert!(shown.contains("<redacted>"));
        assert_eq!(report.outcome, "administrator_issued");
    }

    #[test]
    fn the_administrator_round_trips_through_json() {
        let report = DeploymentOutcomeReport::administrator_issued(
            DEPLOYMENT,
            Uuid::from_u128(9),
            InitialAdministrator::new("admin", "hunter2-Secret"),
        );

        let value = serde_json::to_value(&report).expect("serialised");

        assert_eq!(value["administrator"]["username"], "admin");
        assert_eq!(value["administrator"]["password"], "hunter2-Secret");
        assert_eq!(value["request_id"], serde_json::json!(Uuid::from_u128(9)));
        let back: DeploymentOutcomeReport = serde_json::from_value(value).expect("read back");
        assert_eq!(back, report);
    }

    #[test]
    fn a_report_without_an_administrator_serialises_without_the_key() {
        let value =
            serde_json::to_value(DeploymentOutcomeReport::serving(DEPLOYMENT)).expect("json");

        assert!(value.get("administrator").is_none());
        let old: DeploymentOutcomeReport = serde_json::from_value(
            serde_json::json!({"deployment_id": DEPLOYMENT, "outcome": "running"}),
        )
        .expect("read");
        assert!(old.administrator.is_none());
    }
}
