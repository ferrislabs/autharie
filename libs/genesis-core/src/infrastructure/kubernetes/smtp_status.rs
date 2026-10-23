use std::collections::HashMap;
use std::time::{Duration, Instant};

use autharie_crds::v1alpha::identity_instance::{IdentityInstanceStatus, SmtpPhase, SmtpStatus};
use uuid::Uuid;

use crate::domain::entities::smtp_outcome::{SmtpOutcomeItem, SmtpOutcomeReport};

pub const REPORT_AGAIN_AFTER: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    smtp: Vec<SmtpStatus>,
}

impl Observation {
    pub fn of(status: Option<&IdentityInstanceStatus>) -> Self {
        Self {
            smtp: status.map(|status| status.smtp.clone()).unwrap_or_default(),
        }
    }

    pub fn entries(&self) -> &[SmtpStatus] {
        &self.smtp
    }
}

struct Published {
    status: SmtpStatus,
    at: Instant,
}

#[derive(Default)]
pub struct SmtpTracker {
    published: HashMap<Uuid, HashMap<String, Published>>,
}

pub fn is_due(previous: Option<(&SmtpStatus, Duration)>, status: &SmtpStatus) -> bool {
    match previous {
        None => true,
        Some((last, _)) if last != status => true,
        Some((_, since)) => since >= REPORT_AGAIN_AFTER,
    }
}

impl SmtpTracker {
    pub fn due<'a>(
        &self,
        deployment_id: Uuid,
        observation: &'a Observation,
    ) -> Vec<&'a SmtpStatus> {
        let published = self.published.get(&deployment_id);

        observation
            .entries()
            .iter()
            .filter(|status| {
                let previous = published
                    .and_then(|realms| realms.get(&status.realm))
                    .map(|published| (&published.status, published.at.elapsed()));

                is_due(previous, status)
            })
            .collect()
    }

    pub fn record(&mut self, deployment_id: Uuid, status: &SmtpStatus) {
        self.published.entry(deployment_id).or_default().insert(
            status.realm.clone(),
            Published {
                status: status.clone(),
                at: Instant::now(),
            },
        );
    }

    pub fn retain_observed(&mut self, deployment_id: Uuid, observation: &Observation) {
        if let Some(realms) = self.published.get_mut(&deployment_id) {
            realms.retain(|realm, _| {
                observation
                    .entries()
                    .iter()
                    .any(|status| &status.realm == realm)
            });
        }
    }
}

pub fn build_report(deployment_id: Uuid, status: &SmtpStatus) -> SmtpOutcomeReport {
    SmtpOutcomeReport::new(
        deployment_id,
        status.revision,
        status.realm.as_str(),
        SmtpOutcomeItem {
            phase: match status.phase {
                SmtpPhase::Pending => "Pending",
                SmtpPhase::Applied => "Applied",
                SmtpPhase::Failed => "Failed",
            }
            .to_string(),
            message: status.message.clone(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    fn status(realm: &str, revision: u64, phase: SmtpPhase) -> SmtpStatus {
        SmtpStatus {
            realm: realm.to_string(),
            revision,
            phase,
            message: None,
        }
    }

    fn observation(entries: Vec<SmtpStatus>) -> Observation {
        Observation::of(Some(&IdentityInstanceStatus {
            smtp: entries,
            ..Default::default()
        }))
    }

    fn realms(due: &[&SmtpStatus]) -> Vec<String> {
        due.iter().map(|status| status.realm.clone()).collect()
    }

    #[test]
    fn an_instance_with_no_smtp_status_has_nothing_to_report() {
        let tracker = SmtpTracker::default();

        assert!(tracker.due(DEPLOYMENT, &Observation::of(None)).is_empty());
        assert!(tracker.due(DEPLOYMENT, &observation(Vec::new())).is_empty());
    }

    #[test]
    fn a_first_observation_reports_every_realm() {
        let seen = observation(vec![
            status("master", 1, SmtpPhase::Pending),
            status("acme", 3, SmtpPhase::Applied),
        ]);

        let due = SmtpTracker::default().due(DEPLOYMENT, &seen).len();

        assert_eq!(due, 2);
    }

    #[test]
    fn only_the_realm_that_changed_is_reported_again() {
        let mut tracker = SmtpTracker::default();
        let first = observation(vec![
            status("master", 1, SmtpPhase::Applied),
            status("acme", 3, SmtpPhase::Applied),
        ]);
        for entry in first.entries() {
            tracker.record(DEPLOYMENT, entry);
        }

        let next = observation(vec![
            status("master", 1, SmtpPhase::Applied),
            status("acme", 4, SmtpPhase::Pending),
        ]);

        assert_eq!(realms(&tracker.due(DEPLOYMENT, &next)), ["acme"]);
    }

    #[test]
    fn a_phase_a_revision_or_a_message_changing_is_reported_at_once() {
        let base = status("master", 1, SmtpPhase::Pending);
        let quiet = Duration::from_secs(1);

        let mut phase = base.clone();
        phase.phase = SmtpPhase::Applied;
        let mut revision = base.clone();
        revision.revision = 2;
        let mut message = base.clone();
        message.message = Some("refused".to_string());

        for changed in [phase, revision, message] {
            assert!(is_due(Some((&base, quiet)), &changed));
        }
    }

    #[test]
    fn an_unchanged_status_is_not_republished_until_the_window_passes() {
        let mut tracker = SmtpTracker::default();
        let seen = observation(vec![status("master", 1, SmtpPhase::Applied)]);
        tracker.record(DEPLOYMENT, &seen.entries()[0]);

        assert!(tracker.due(DEPLOYMENT, &seen).is_empty());
        assert!(is_due(
            Some((&seen.entries()[0], REPORT_AGAIN_AFTER)),
            &seen.entries()[0]
        ));
        assert!(!is_due(
            Some((&seen.entries()[0], Duration::from_secs(1))),
            &seen.entries()[0]
        ));
    }

    #[test]
    fn a_report_that_failed_to_publish_stays_owed() {
        let tracker = SmtpTracker::default();
        let seen = observation(vec![status("master", 1, SmtpPhase::Applied)]);

        assert_eq!(tracker.due(DEPLOYMENT, &seen).len(), 1);
        assert_eq!(tracker.due(DEPLOYMENT, &seen).len(), 1);
    }

    #[test]
    fn deployments_are_tracked_apart() {
        let mut tracker = SmtpTracker::default();
        let seen = observation(vec![status("master", 1, SmtpPhase::Applied)]);
        tracker.record(DEPLOYMENT, &seen.entries()[0]);

        assert_eq!(tracker.due(Uuid::from_u128(2), &seen).len(), 1);
        assert!(tracker.due(DEPLOYMENT, &seen).is_empty());
    }

    #[test]
    fn a_realm_that_left_the_status_is_reported_again_if_it_comes_back() {
        let mut tracker = SmtpTracker::default();
        let seen = observation(vec![status("master", 1, SmtpPhase::Applied)]);
        tracker.record(DEPLOYMENT, &seen.entries()[0]);

        tracker.retain_observed(DEPLOYMENT, &Observation::default());

        assert_eq!(tracker.due(DEPLOYMENT, &seen).len(), 1);
    }

    #[test]
    fn the_report_is_the_frozen_outcome_with_one_item() {
        let report = build_report(DEPLOYMENT, &status("master", 4, SmtpPhase::Applied));

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "smtp",
                "revision": 4,
                "realm": "master",
                "items": [{ "phase": "Applied" }],
            })
        );
    }

    #[test]
    fn each_phase_and_the_message_come_through() {
        for (phase, spelled) in [
            (SmtpPhase::Pending, "Pending"),
            (SmtpPhase::Applied, "Applied"),
            (SmtpPhase::Failed, "Failed"),
        ] {
            let mut entry = status("acme", 2, phase);
            entry.message = Some("because".to_string());

            let value = serde_json::to_value(build_report(DEPLOYMENT, &entry)).expect("serialises");

            assert_eq!(
                value["items"],
                json!([{ "phase": spelled, "message": "because" }])
            );
            assert_eq!(value["revision"], json!(2));
            assert_eq!(value["realm"], json!("acme"));
        }
    }

    #[test]
    fn the_status_of_an_instance_becomes_an_observation() {
        let status = IdentityInstanceStatus {
            smtp: vec![status("master", 1, SmtpPhase::Applied)],
            ..Default::default()
        };

        assert_eq!(Observation::of(Some(&status)).entries().len(), 1);
        assert_eq!(Observation::of(None), Observation::default());
    }
}
