use std::collections::{HashMap, HashSet};

use autharie_crds::v1alpha::identity_instance::{
    IamRequestPhase, IamRequestSpec, IamRequestStatus, IdentityInstanceStatus,
};
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::domain::entities::iam_outcome::IamOutcomeReport;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::ports::{IamOutcomePublisher, IamPort, IamResultsPort};

#[derive(Default)]
pub struct IamTracker {
    published: HashMap<Uuid, HashSet<String>>,
}

impl IamTracker {
    pub fn has_published(&self, deployment_id: Uuid, request_id: &str) -> bool {
        self.published
            .get(&deployment_id)
            .is_some_and(|ids| ids.contains(request_id))
    }

    pub fn record(&mut self, deployment_id: Uuid, request_id: &str) {
        self.published
            .entry(deployment_id)
            .or_default()
            .insert(request_id.to_string());
    }

    pub fn retain_asked(&mut self, deployment_id: Uuid, asked: &[IamRequestSpec]) {
        if let Some(ids) = self.published.get_mut(&deployment_id) {
            ids.retain(|id| asked.iter().any(|request| &request.request_id == id));
        }
    }
}

pub fn finished<'a>(
    asked: &[IamRequestSpec],
    status: Option<&'a IdentityInstanceStatus>,
) -> Vec<&'a IamRequestStatus> {
    let recorded = status
        .and_then(|status| status.iam.as_ref())
        .map(|iam| iam.requests.as_slice())
        .unwrap_or_default();

    asked
        .iter()
        .filter_map(|request| {
            recorded
                .iter()
                .find(|status| status.request_id == request.request_id)
        })
        .collect()
}

pub fn report_for(
    deployment_id: Uuid,
    status: &IamRequestStatus,
    stored: Option<Value>,
) -> Option<IamOutcomeReport> {
    match (status.phase, stored) {
        (IamRequestPhase::Failed, _) => Some(IamOutcomeReport::failed(
            deployment_id,
            status.request_id.as_str(),
            status.message.clone(),
        )),
        (IamRequestPhase::Done, Some(envelope)) => Some(IamOutcomeReport::done(
            deployment_id,
            status.request_id.as_str(),
            envelope,
        )),
        (IamRequestPhase::Done, None) => None,
    }
}

pub async fn settle<I, P>(
    iam: &I,
    publisher: &P,
    reference: &IdentityInstanceRef,
    deployment_id: Uuid,
    finished: &[&IamRequestStatus],
    tracker: &mut IamTracker,
) where
    I: IamPort + IamResultsPort,
    P: IamOutcomePublisher,
{
    let mut settled: Vec<&str> = Vec::with_capacity(finished.len());

    for status in finished {
        let id = status.request_id.as_str();

        if !tracker.has_published(deployment_id, id) {
            let stored = match status.phase {
                IamRequestPhase::Done => match iam.read(reference, id).await {
                    Ok(stored) => stored,
                    Err(err) => {
                        warn!(%err, %deployment_id, request_id = id, "failed to read an IAM result");
                        continue;
                    }
                },
                IamRequestPhase::Failed => None,
            };

            if let Some(report) = report_for(deployment_id, status, stored)
                && let Err(err) = publisher.publish_iam(&report).await
            {
                warn!(%err, %deployment_id, request_id = id, "failed to publish an IAM outcome");
                continue;
            }
            tracker.record(deployment_id, id);
        }

        settled.push(id);
    }

    if settled.is_empty() {
        return;
    }
    if let Err(err) = iam.forget(reference, &settled).await {
        warn!(%err, %deployment_id, "failed to drop published IAM results");
        return;
    }
    if let Err(err) = iam.drop_requests(reference, &settled).await {
        warn!(%err, %deployment_id, "failed to prune published IAM requests");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use autharie_crds::v1alpha::identity_instance::IamStatus;
    use serde_json::json;

    use super::*;
    use crate::domain::error::GenesisError;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const SEALED: &str = "sealed-ciphertext";

    fn asked(id: &str) -> IamRequestSpec {
        IamRequestSpec {
            request_id: id.to_string(),
            realm: "master".to_string(),
            operation: "{}".to_string(),
            sealing_key: "{}".to_string(),
            expires_at: "2026-10-24T12:05:00Z".to_string(),
        }
    }

    fn done(id: &str) -> IamRequestStatus {
        IamRequestStatus {
            request_id: id.to_string(),
            phase: IamRequestPhase::Done,
            message: None,
        }
    }

    fn failed(id: &str, message: &str) -> IamRequestStatus {
        IamRequestStatus {
            request_id: id.to_string(),
            phase: IamRequestPhase::Failed,
            message: Some(message.to_string()),
        }
    }

    fn envelope() -> Value {
        json!({ "v": 1, "alg": "RSA-OAEP-256+A256GCM", "ek": SEALED, "iv": "b", "ct": "c" })
    }

    fn reference() -> IdentityInstanceRef {
        IdentityInstanceRef::for_deployment(DEPLOYMENT, "tenant-a")
    }

    #[derive(Default)]
    struct World {
        stored: Mutex<HashMap<String, Value>>,
        log: Mutex<Vec<String>>,
        published: Mutex<Vec<IamOutcomeReport>>,
        fail_publish: Mutex<bool>,
        fail_forget: Mutex<bool>,
        fail_read: Mutex<bool>,
    }

    impl World {
        fn with_result(id: &str) -> Self {
            let world = Self::default();
            world
                .stored
                .lock()
                .unwrap()
                .insert(id.to_string(), envelope());
            world
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        fn note(&self, line: String) {
            self.log.lock().unwrap().push(line);
        }
    }

    impl IamPort for World {
        async fn request<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            _request: &'a crate::domain::entities::iam_payload::IamRequestEntry,
        ) -> Result<(), GenesisError> {
            Ok(())
        }

        async fn drop_requests<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            request_ids: &'a [&'a str],
        ) -> Result<(), GenesisError> {
            self.note(format!("drop {}", request_ids.join(",")));
            Ok(())
        }
    }

    impl IamResultsPort for World {
        async fn read<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            request_id: &'a str,
        ) -> Result<Option<Value>, GenesisError> {
            self.note(format!("read {request_id}"));
            if *self.fail_read.lock().unwrap() {
                return Err(GenesisError::Kubernetes {
                    message: "down".to_string(),
                });
            }
            Ok(self.stored.lock().unwrap().get(request_id).cloned())
        }

        async fn forget<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            request_ids: &'a [&'a str],
        ) -> Result<(), GenesisError> {
            if *self.fail_forget.lock().unwrap() {
                self.note("forget failed".to_string());
                return Err(GenesisError::Kubernetes {
                    message: "down".to_string(),
                });
            }
            let mut stored = self.stored.lock().unwrap();
            for id in request_ids {
                stored.remove(*id);
            }
            drop(stored);
            self.note(format!("forget {}", request_ids.join(",")));
            Ok(())
        }
    }

    impl IamOutcomePublisher for World {
        async fn publish_iam<'a>(
            &'a self,
            report: &'a IamOutcomeReport,
        ) -> Result<(), GenesisError> {
            if *self.fail_publish.lock().unwrap() {
                self.note("publish failed".to_string());
                return Err(GenesisError::MessageBus {
                    message: "down".to_string(),
                });
            }
            self.note(format!("publish {}", report.request_id));
            self.published.lock().unwrap().push(report.clone());
            Ok(())
        }
    }

    async fn run(world: &World, tracker: &mut IamTracker, statuses: &[IamRequestStatus]) {
        let refs: Vec<&IamRequestStatus> = statuses.iter().collect();
        settle(world, world, &reference(), DEPLOYMENT, &refs, tracker).await;
    }

    #[tokio::test]
    async fn a_done_request_is_published_with_its_envelope_then_its_key_and_entry_are_removed() {
        let world = World::with_result("r1");

        run(&world, &mut IamTracker::default(), &[done("r1")]).await;

        assert_eq!(
            world.log(),
            ["read r1", "publish r1", "forget r1", "drop r1"]
        );
        let published = world.published.lock().unwrap();
        assert_eq!(published[0].envelope.as_ref(), Some(&envelope()));
        assert_eq!(published[0].message, None);
        assert!(world.stored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_request_is_published_with_its_message_and_no_envelope() {
        let world = World::default();

        run(
            &world,
            &mut IamTracker::default(),
            &[failed("r1", "result too large")],
        )
        .await;

        assert_eq!(world.log(), ["publish r1", "forget r1", "drop r1"]);
        let published = world.published.lock().unwrap();
        assert_eq!(published[0].message.as_deref(), Some("result too large"));
        assert_eq!(published[0].envelope, None);
    }

    #[tokio::test]
    async fn a_crash_between_publish_and_delete_republishes_and_then_cleans_up() {
        let world = World::with_result("r1");
        *world.fail_forget.lock().unwrap() = true;
        let mut tracker = IamTracker::default();

        run(&world, &mut tracker, &[done("r1")]).await;
        assert_eq!(world.log(), ["read r1", "publish r1", "forget failed"]);
        assert_eq!(world.stored.lock().unwrap().len(), 1);

        let mut restarted = IamTracker::default();
        *world.fail_forget.lock().unwrap() = false;
        run(&world, &mut restarted, &[done("r1")]).await;

        assert_eq!(world.published.lock().unwrap().len(), 2);
        assert!(world.stored.lock().unwrap().is_empty());
        assert_eq!(world.log().last().map(String::as_str), Some("drop r1"));
    }

    #[tokio::test]
    async fn the_same_process_does_not_publish_twice_while_cleanup_is_failing() {
        let world = World::with_result("r1");
        *world.fail_forget.lock().unwrap() = true;
        let mut tracker = IamTracker::default();

        run(&world, &mut tracker, &[done("r1")]).await;
        run(&world, &mut tracker, &[done("r1")]).await;
        *world.fail_forget.lock().unwrap() = false;
        run(&world, &mut tracker, &[done("r1")]).await;

        assert_eq!(world.published.lock().unwrap().len(), 1);
        assert!(world.stored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_crash_between_deleting_the_key_and_pruning_the_entry_publishes_nothing_more() {
        let world = World::default();

        run(&world, &mut IamTracker::default(), &[done("r1")]).await;

        assert!(world.published.lock().unwrap().is_empty());
        assert_eq!(world.log(), ["read r1", "forget r1", "drop r1"]);
    }

    #[tokio::test]
    async fn an_outcome_that_cannot_be_published_keeps_the_key_and_the_entry() {
        let world = World::with_result("r1");
        *world.fail_publish.lock().unwrap() = true;
        let mut tracker = IamTracker::default();

        run(&world, &mut tracker, &[done("r1")]).await;

        assert_eq!(world.log(), ["read r1", "publish failed"]);
        assert_eq!(world.stored.lock().unwrap().len(), 1);
        assert!(!tracker.has_published(DEPLOYMENT, "r1"));

        *world.fail_publish.lock().unwrap() = false;
        run(&world, &mut tracker, &[done("r1")]).await;

        assert_eq!(world.published.lock().unwrap().len(), 1);
        assert!(world.stored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_result_that_cannot_be_read_is_neither_published_as_missing_nor_dropped() {
        let world = World::with_result("r1");
        *world.fail_read.lock().unwrap() = true;

        run(&world, &mut IamTracker::default(), &[done("r1")]).await;

        assert_eq!(world.log(), ["read r1"]);
        assert_eq!(world.stored.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn each_request_settles_on_its_own() {
        let world = World::with_result("r2");
        *world.fail_read.lock().unwrap() = false;

        run(
            &world,
            &mut IamTracker::default(),
            &[failed("r1", "no"), done("r2")],
        )
        .await;

        let ids: Vec<_> = world
            .published
            .lock()
            .unwrap()
            .iter()
            .map(|report| report.request_id.clone())
            .collect();
        assert_eq!(ids, ["r1", "r2"]);
        assert_eq!(world.log().last().map(String::as_str), Some("drop r1,r2"));
    }

    #[tokio::test]
    async fn nothing_finished_touches_nothing() {
        let world = World::default();

        run(&world, &mut IamTracker::default(), &[]).await;

        assert!(world.log().is_empty());
    }

    #[test]
    fn only_requests_still_asked_for_and_already_recorded_are_finished() {
        let status = IdentityInstanceStatus {
            iam: Some(IamStatus {
                phase: None,
                message: None,
                applied: None,
                observed_at: None,
                requests: vec![done("r1"), done("old"), failed("r3", "no")],
            }),
            ..Default::default()
        };

        let ready = finished(&[asked("r1"), asked("r2"), asked("r3")], Some(&status));

        let ids: Vec<&str> = ready.iter().map(|s| s.request_id.as_str()).collect();
        assert_eq!(ids, ["r1", "r3"]);
        assert!(finished(&[asked("r1")], None).is_empty());
        assert!(finished(&[asked("r1")], Some(&IdentityInstanceStatus::default())).is_empty());
    }

    #[test]
    fn the_tracker_forgets_requests_that_left_the_spec_and_keeps_deployments_apart() {
        let mut tracker = IamTracker::default();
        tracker.record(DEPLOYMENT, "r1");
        tracker.record(DEPLOYMENT, "r2");
        tracker.record(Uuid::from_u128(2), "r1");

        tracker.retain_asked(DEPLOYMENT, &[asked("r2")]);

        assert!(!tracker.has_published(DEPLOYMENT, "r1"));
        assert!(tracker.has_published(DEPLOYMENT, "r2"));
        assert!(tracker.has_published(Uuid::from_u128(2), "r1"));
    }

    #[test]
    fn the_report_follows_the_phase() {
        assert_eq!(
            report_for(DEPLOYMENT, &done("r1"), Some(envelope()))
                .unwrap()
                .envelope,
            Some(envelope())
        );
        assert_eq!(report_for(DEPLOYMENT, &done("r1"), None), None);
        assert!(
            report_for(DEPLOYMENT, &failed("r1", "x"), Some(envelope()))
                .unwrap()
                .envelope
                .is_none()
        );
    }
}
