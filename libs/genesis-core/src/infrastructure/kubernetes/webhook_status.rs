use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstanceStatus, WebhookPhase, WebhookRequestPhase, WebhookRequestStatus, WebhookStatus,
};
use uuid::Uuid;

use crate::domain::entities::webhooks_outcome::{
    RequestOutcome, SecretOutcome, WebhookOutcome, WebhooksOutcomeReport,
};

pub const REPORT_AGAIN_AFTER: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    webhooks: Vec<WebhookStatus>,
    realms: Vec<String>,
    requests: Vec<WebhookRequestStatus>,
}

impl Observation {
    pub fn of(status: Option<&IdentityInstanceStatus>) -> Self {
        match status {
            Some(status) => Self {
                webhooks: status.webhooks.clone(),
                realms: status.realms.clone(),
                requests: status.webhook_requests.clone(),
            },
            None => Self::default(),
        }
    }

    fn is_empty(&self) -> bool {
        self.webhooks.is_empty() && self.realms.is_empty() && self.requests.is_empty()
    }
}

struct Published {
    observation: Observation,
    at: Instant,
    versions: HashMap<String, u64>,
    requests: HashSet<String>,
}

#[derive(Default)]
pub struct WebhookTracker {
    published: HashMap<Uuid, Published>,
}

pub fn is_due(previous: Option<(&Observation, Duration)>, observation: &Observation) -> bool {
    match previous {
        None => !observation.is_empty(),
        Some((last, _)) if last != observation => true,
        Some((_, since)) => !observation.is_empty() && since >= REPORT_AGAIN_AFTER,
    }
}

impl WebhookTracker {
    pub fn is_due(&self, deployment_id: Uuid, observation: &Observation) -> bool {
        let previous = self
            .published
            .get(&deployment_id)
            .map(|published| (&published.observation, published.at.elapsed()));

        is_due(previous, observation)
    }

    fn last_version(&self, deployment_id: Uuid, webhook: &str) -> u64 {
        self.published
            .get(&deployment_id)
            .and_then(|published| published.versions.get(webhook))
            .copied()
            .unwrap_or(0)
    }

    pub fn needs_secrets(&self, deployment_id: Uuid, observation: &Observation) -> bool {
        observation
            .webhooks
            .iter()
            .any(|webhook| webhook.secret_version > self.last_version(deployment_id, &webhook.id))
    }

    pub fn record(
        &mut self,
        deployment_id: Uuid,
        observation: Observation,
        report: &WebhooksOutcomeReport,
    ) {
        let mut versions = self
            .published
            .remove(&deployment_id)
            .map(|published| published.versions)
            .unwrap_or_default();

        versions.retain(|id, _| observation.webhooks.iter().any(|webhook| &webhook.id == id));
        for secret in &report.secrets {
            let version = versions.entry(secret.id.clone()).or_insert(0);
            *version = (*version).max(secret.version);
        }

        let requests = report
            .requests
            .iter()
            .map(|request| request.id.clone())
            .collect();

        self.published.insert(
            deployment_id,
            Published {
                observation,
                at: Instant::now(),
                versions,
                requests,
            },
        );
    }

    pub fn published_requests(&self, deployment_id: Uuid) -> Vec<&str> {
        self.published
            .get(&deployment_id)
            .map(|published| published.requests.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }
}

pub fn build_report(
    deployment_id: Uuid,
    revision: u64,
    observation: &Observation,
    tracker: &WebhookTracker,
    secret_values: &BTreeMap<String, String>,
) -> WebhooksOutcomeReport {
    let webhooks = observation
        .webhooks
        .iter()
        .map(|webhook| WebhookOutcome {
            id: webhook.id.clone(),
            ferriskey_id: webhook.ferriskey_id.clone(),
            phase: match webhook.phase {
                WebhookPhase::Pending => "Pending",
                WebhookPhase::Active => "Active",
                WebhookPhase::Failed => "Failed",
            }
            .to_string(),
            message: webhook.message.clone(),
            secret_version: webhook.secret_version,
        })
        .collect();

    let secrets = observation
        .webhooks
        .iter()
        .filter(|webhook| webhook.secret_version > tracker.last_version(deployment_id, &webhook.id))
        .filter_map(|webhook| {
            secret_values.get(&webhook.id).map(|secret| SecretOutcome {
                id: webhook.id.clone(),
                version: webhook.secret_version,
                secret: secret.clone(),
            })
        })
        .collect();

    let requests = observation
        .requests
        .iter()
        .map(|request| RequestOutcome {
            id: request.id.clone(),
            phase: match request.phase {
                WebhookRequestPhase::Done => "Done",
                WebhookRequestPhase::Failed => "Failed",
            }
            .to_string(),
            message: request.message.clone(),
        })
        .collect();

    WebhooksOutcomeReport::new(
        deployment_id,
        revision,
        webhooks,
        secrets,
        observation.realms.clone(),
        requests,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    fn active(id: &str, version: u64) -> WebhookStatus {
        WebhookStatus {
            id: id.to_string(),
            ferriskey_id: Some(format!("fk-{id}")),
            phase: WebhookPhase::Active,
            message: None,
            secret_version: version,
        }
    }

    fn done(id: &str) -> WebhookRequestStatus {
        WebhookRequestStatus {
            id: id.to_string(),
            phase: WebhookRequestPhase::Done,
            message: None,
        }
    }

    fn observation(webhooks: Vec<WebhookStatus>) -> Observation {
        Observation {
            webhooks,
            realms: vec!["master".to_string()],
            requests: Vec::new(),
        }
    }

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(id, secret)| (id.to_string(), secret.to_string()))
            .collect()
    }

    #[test]
    fn an_instance_with_no_webhook_status_is_never_reported() {
        assert!(!is_due(None, &Observation::default()));
        assert!(!is_due(
            Some((&Observation::default(), REPORT_AGAIN_AFTER)),
            &Observation::default()
        ));
    }

    #[test]
    fn a_first_observation_is_reported() {
        assert!(is_due(None, &observation(vec![active("a", 0)])));
    }

    #[test]
    fn a_change_in_any_of_the_three_status_fields_is_reported_at_once() {
        let base = observation(vec![active("a", 1)]);
        let quiet = Duration::from_secs(1);

        let mut webhooks = base.clone();
        webhooks.webhooks[0].phase = WebhookPhase::Failed;
        let mut realms = base.clone();
        realms.realms.push("other".to_string());
        let mut requests = base.clone();
        requests.requests.push(done("r1"));

        for changed in [webhooks, realms, requests] {
            assert!(is_due(Some((&base, quiet)), &changed));
        }
    }

    #[test]
    fn everything_going_away_is_a_change_worth_reporting() {
        let before = observation(vec![active("a", 1)]);

        assert!(is_due(
            Some((&before, Duration::from_secs(1))),
            &Observation::default()
        ));
    }

    #[test]
    fn an_unchanged_observation_is_quiet_until_the_window_passes() {
        let seen = observation(vec![active("a", 1)]);

        assert!(!is_due(Some((&seen, Duration::from_secs(1))), &seen));
        assert!(is_due(Some((&seen, REPORT_AGAIN_AFTER)), &seen));
    }

    #[test]
    fn the_report_carries_the_frozen_fields_from_the_status() {
        let mut seen = observation(vec![
            active("a", 0),
            WebhookStatus {
                id: "b".to_string(),
                ferriskey_id: None,
                phase: WebhookPhase::Failed,
                message: Some("ssrf".to_string()),
                secret_version: 0,
            },
        ]);
        seen.requests = vec![WebhookRequestStatus {
            id: "r".to_string(),
            phase: WebhookRequestPhase::Failed,
            message: Some("gone".to_string()),
        }];

        let report = build_report(
            DEPLOYMENT,
            12,
            &seen,
            &WebhookTracker::default(),
            &BTreeMap::new(),
        );
        let json = serde_json::to_value(&report).expect("serialises");

        assert_eq!(json["kind"], "webhooks");
        assert_eq!(json["revision"], 12);
        assert_eq!(json["webhooks"][0]["phase"], "Active");
        assert_eq!(json["webhooks"][0]["ferriskey_id"], "fk-a");
        assert_eq!(json["webhooks"][1]["phase"], "Failed");
        assert_eq!(json["webhooks"][1]["message"], "ssrf");
        assert!(json["webhooks"][1].get("ferriskey_id").is_none());
        assert_eq!(json["realms"][0], "master");
        assert_eq!(json["requests"][0]["phase"], "Failed");
        assert_eq!(json["secrets"], serde_json::json!([]));
    }

    #[test]
    fn a_secret_is_included_the_first_time_its_version_is_seen() {
        let seen = observation(vec![active("a", 1)]);
        let tracker = WebhookTracker::default();

        assert!(tracker.needs_secrets(DEPLOYMENT, &seen));
        let report = build_report(DEPLOYMENT, 1, &seen, &tracker, &values(&[("a", "s1")]));

        assert_eq!(report.secrets.len(), 1);
        assert_eq!(report.secrets[0].id, "a");
        assert_eq!(report.secrets[0].version, 1);
        assert_eq!(report.secrets[0].secret, "s1");
    }

    #[test]
    fn a_published_version_is_not_sent_again_but_a_higher_one_is() {
        let mut tracker = WebhookTracker::default();
        let first = observation(vec![active("a", 1)]);
        let report = build_report(DEPLOYMENT, 1, &first, &tracker, &values(&[("a", "s1")]));
        tracker.record(DEPLOYMENT, first.clone(), &report);

        assert!(!tracker.needs_secrets(DEPLOYMENT, &first));
        let unchanged = build_report(DEPLOYMENT, 1, &first, &tracker, &values(&[("a", "s1")]));
        assert!(unchanged.secrets.is_empty());

        let rotated = observation(vec![active("a", 2)]);
        assert!(tracker.needs_secrets(DEPLOYMENT, &rotated));
        let report = build_report(DEPLOYMENT, 1, &rotated, &tracker, &values(&[("a", "s2")]));
        assert_eq!(report.secrets.len(), 1);
        assert_eq!(report.secrets[0].version, 2);
        assert_eq!(report.secrets[0].secret, "s2");
    }

    #[test]
    fn only_the_webhook_whose_version_rose_carries_a_secret() {
        let mut tracker = WebhookTracker::default();
        let first = observation(vec![active("a", 1), active("b", 1)]);
        let report = build_report(
            DEPLOYMENT,
            1,
            &first,
            &tracker,
            &values(&[("a", "sa"), ("b", "sb")]),
        );
        assert_eq!(report.secrets.len(), 2);
        tracker.record(DEPLOYMENT, first, &report);

        let next = observation(vec![active("a", 1), active("b", 2)]);
        let report = build_report(
            DEPLOYMENT,
            1,
            &next,
            &tracker,
            &values(&[("a", "sa"), ("b", "sb2")]),
        );

        assert_eq!(report.secrets.len(), 1);
        assert_eq!(report.secrets[0].id, "b");
    }

    #[test]
    fn a_secret_that_could_not_be_read_stays_owed_to_the_next_report() {
        let mut tracker = WebhookTracker::default();
        let seen = observation(vec![active("a", 1)]);

        let report = build_report(DEPLOYMENT, 1, &seen, &tracker, &BTreeMap::new());
        assert!(report.secrets.is_empty());
        tracker.record(DEPLOYMENT, seen.clone(), &report);

        assert!(tracker.needs_secrets(DEPLOYMENT, &seen));
    }

    #[test]
    fn a_version_zero_has_no_secret_to_send() {
        let seen = observation(vec![active("a", 0)]);
        let tracker = WebhookTracker::default();

        assert!(!tracker.needs_secrets(DEPLOYMENT, &seen));
        assert!(
            build_report(DEPLOYMENT, 1, &seen, &tracker, &values(&[("a", "x")]))
                .secrets
                .is_empty()
        );
    }

    #[test]
    fn deployments_are_tracked_apart() {
        let mut tracker = WebhookTracker::default();
        let seen = observation(vec![active("a", 1)]);
        let report = build_report(DEPLOYMENT, 1, &seen, &tracker, &values(&[("a", "s")]));
        tracker.record(DEPLOYMENT, seen.clone(), &report);

        assert!(tracker.needs_secrets(Uuid::from_u128(2), &seen));
        assert!(tracker.is_due(Uuid::from_u128(2), &seen));
        assert!(!tracker.is_due(DEPLOYMENT, &seen));
    }

    #[test]
    fn the_requests_of_a_published_report_become_prunable() {
        let mut tracker = WebhookTracker::default();
        let mut seen = observation(vec![active("a", 0)]);
        seen.requests = vec![done("r1"), done("r2")];

        assert!(tracker.published_requests(DEPLOYMENT).is_empty());
        let report = build_report(DEPLOYMENT, 1, &seen, &tracker, &BTreeMap::new());
        tracker.record(DEPLOYMENT, seen, &report);

        let mut ids = tracker.published_requests(DEPLOYMENT);
        ids.sort_unstable();
        assert_eq!(ids, ["r1", "r2"]);
    }

    #[test]
    fn the_status_of_an_instance_becomes_an_observation() {
        let status = IdentityInstanceStatus {
            webhooks: vec![active("a", 1)],
            realms: vec!["master".to_string()],
            webhook_requests: vec![done("r")],
            ..Default::default()
        };

        let seen = Observation::of(Some(&status));

        assert_eq!(seen.webhooks.len(), 1);
        assert_eq!(seen.realms, ["master"]);
        assert_eq!(seen.requests.len(), 1);
        assert_eq!(Observation::of(None), Observation::default());
    }
}
