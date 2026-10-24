use std::sync::Arc;

use autharie_crds::v1alpha::identity_instance::{
    IamRequestPhase, IamRequestSpec, IamRequestStatus, IdentityInstance, IdentityProvider,
    SealingKeySpec,
};
use chrono::{DateTime, Utc};

use super::ports::{
    FerriskeyIamPort, IamOutcome, IamResultsPort, IamStatusRepository, IdentityInstanceIam,
};
use super::{IamError, IamOperation, MAX_RESULT_BYTES, RequestId, envelope};
use crate::domain::agents::sealing::SealingKey;

const MAX_RECORDED_REQUESTS: usize = 100;
const RESULT_TOO_LARGE: &str = "result too large";
const REQUEST_EXPIRED: &str = "the request expired before it was run";

pub struct IamReconciler<R, F, K> {
    status: Arc<R>,
    ferriskey: Arc<F>,
    results: Arc<K>,
}

impl<R, F, K> IamReconciler<R, F, K> {
    pub fn new(status: Arc<R>, ferriskey: Arc<F>, results: Arc<K>) -> Self {
        Self {
            status,
            ferriskey,
            results,
        }
    }
}

enum Run {
    Recorded(IamRequestStatus),
    Defer,
}

impl<R, F, K> IdentityInstanceIam for IamReconciler<R, F, K>
where
    R: IamStatusRepository,
    F: FerriskeyIamPort,
    K: IamResultsPort,
{
    async fn reconcile(&self, instance: &IdentityInstance) -> IamOutcome {
        self.reconcile_at(instance, Utc::now()).await
    }
}

impl<R, F, K> IamReconciler<R, F, K>
where
    R: IamStatusRepository,
    F: FerriskeyIamPort,
    K: IamResultsPort,
{
    pub async fn reconcile_at(
        &self,
        instance: &IdentityInstance,
        now: DateTime<Utc>,
    ) -> IamOutcome {
        let asked = &instance.spec.iam_requests;
        if asked.is_empty()
            || instance.spec.provider != IdentityProvider::Ferriskey
            || instance.spec.stopped
            || !instance.is_ready()
        {
            return IamOutcome::Settled;
        }

        let recorded = match self.status.recorded(instance).await {
            Ok(recorded) => recorded,
            Err(error) => {
                tracing::warn!(error = %error, "could not read which IAM requests already ran");
                return IamOutcome::Retry;
            }
        };

        let mut next = recorded.clone();
        let mut retry = false;
        for request in asked {
            if next
                .iter()
                .any(|done| done.request_id == request.request_id)
            {
                continue;
            }
            match self.run(instance, request, now).await {
                Run::Recorded(status) => next.push(status),
                Run::Defer => retry = true,
            }
        }
        trim_recorded(&mut next, asked);

        if next != recorded
            && let Err(error) = self.status.patch_requests(instance, &next).await
        {
            tracing::warn!(error = %error, "could not report the IAM requests");
            retry = true;
        }

        self.forget_expired(instance, asked, &next, now).await;

        if retry {
            IamOutcome::Retry
        } else {
            IamOutcome::Settled
        }
    }

    async fn run(
        &self,
        instance: &IdentityInstance,
        request: &IamRequestSpec,
        now: DateTime<Utc>,
    ) -> Run {
        let record = |phase: IamRequestPhase, message: Option<String>| {
            Run::Recorded(IamRequestStatus {
                request_id: request.request_id.clone(),
                phase,
                message,
            })
        };

        match self.execute(instance, request, now).await {
            Ok(()) => record(IamRequestPhase::Done, None),
            Err(error) if error.is_retry() => {
                tracing::warn!(error = %error.message(), "an IAM request will be tried again");
                Run::Defer
            }
            Err(error) => record(IamRequestPhase::Failed, Some(error.message().to_string())),
        }
    }

    async fn execute(
        &self,
        instance: &IdentityInstance,
        request: &IamRequestSpec,
        now: DateTime<Utc>,
    ) -> Result<(), IamError> {
        let id = RequestId::parse(&request.request_id)?;
        ensure_unexpired(&request.expires_at, now)?;
        let operation = IamOperation::try_from(request.operation.as_str())?;
        let key = sealing_key(&request.sealing_key)?;

        let result = self
            .ferriskey
            .execute(instance, &request.realm, &operation)
            .await?;

        let plaintext = result
            .to_json()
            .map_err(|_| IamError::rejected("the result could not be written"))?;
        if plaintext.len() > MAX_RESULT_BYTES {
            return Err(IamError::rejected(RESULT_TOO_LARGE));
        }

        let sealed = envelope::seal(&key, &id, &plaintext)
            .map_err(|_| IamError::rejected("the result could not be sealed"))?;
        let sealed = serde_json::to_string(&sealed)
            .map_err(|_| IamError::rejected("the result could not be sealed"))?;

        self.results
            .put(instance, &request.request_id, &sealed)
            .await
    }

    async fn forget_expired(
        &self,
        instance: &IdentityInstance,
        asked: &[IamRequestSpec],
        recorded: &[IamRequestStatus],
        now: DateTime<Utc>,
    ) {
        let stale: Vec<&str> = asked
            .iter()
            .filter(|request| is_expired(&request.expires_at, now))
            .filter(|request| {
                recorded.iter().any(|done| {
                    done.request_id == request.request_id && done.phase == IamRequestPhase::Done
                })
            })
            .map(|request| request.request_id.as_str())
            .collect();

        if stale.is_empty() {
            return;
        }
        if let Err(error) = self.results.forget(instance, &stale).await {
            tracing::warn!(error = %error.message(), "could not drop the results of expired IAM requests");
        }
    }
}

fn is_expired(expires_at: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(expires_at).is_ok_and(|expiry| expiry <= now)
}

fn ensure_unexpired(expires_at: &str, now: DateTime<Utc>) -> Result<(), IamError> {
    let expiry = DateTime::parse_from_rfc3339(expires_at)
        .map_err(|_| IamError::rejected("the request has no valid expiry"))?;
    if expiry <= now {
        return Err(IamError::rejected(REQUEST_EXPIRED));
    }
    Ok(())
}

fn sealing_key(raw: &str) -> Result<SealingKey, IamError> {
    let spec: SealingKeySpec = serde_json::from_str(raw)
        .map_err(|_| IamError::rejected("the sealing key is not well formed"))?;
    SealingKey::try_from(&spec).map_err(|error| IamError::rejected(error.to_string()))
}

fn trim_recorded(recorded: &mut Vec<IamRequestStatus>, asked: &[IamRequestSpec]) {
    while recorded.len() > MAX_RECORDED_REQUESTS {
        match recorded.iter().position(|done| {
            !asked
                .iter()
                .any(|request| request.request_id == done.request_id)
        }) {
            Some(oldest) => {
                recorded.remove(oldest);
            }
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use autharie_crds::common::types::{Phase, ResourceRequirements};
    use autharie_crds::v1alpha::identity_instance::{
        DatabaseConfig, DatabaseMode, IdentityInstanceSpec, IdentityInstanceStatus,
        ManagedClusterConfig, ManagedClusterStorage,
    };
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use chrono::TimeZone;
    use kube::core::ObjectMeta;
    use serde_json::{Value, json};

    use super::*;
    use crate::domain::OperatorError;
    use crate::domain::agents::sealing::fixtures::{private_key, public_spec};
    use crate::domain::iam::PageSize;
    use crate::domain::iam::envelope::{Envelope, fixtures::open};
    use crate::domain::iam::result::{
        ApplicationPage, IamResult, fixtures::detail, fixtures::summary,
    };

    const FIRST: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const SECOND: &str = "7f1c2d3e-0000-4000-8000-00000000000b";
    const APP: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a22";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap()
    }

    fn list_operation() -> String {
        json!({ "type": "applications.list", "page": 1, "limit": 25 }).to_string()
    }

    fn key_json() -> String {
        serde_json::to_string(&json!({
            "kty": "RSA",
            "alg": "RSA-OAEP-256",
            "n": public_spec().n,
            "e": public_spec().e,
        }))
        .unwrap()
    }

    fn request(id: &str) -> IamRequestSpec {
        IamRequestSpec {
            request_id: id.to_string(),
            realm: "master".to_string(),
            operation: list_operation(),
            sealing_key: key_json(),
            expires_at: "2026-10-24T12:05:00Z".to_string(),
        }
    }

    fn instance(requests: Vec<IamRequestSpec>) -> IdentityInstance {
        IdentityInstance {
            metadata: ObjectMeta {
                name: Some("acme".to_string()),
                namespace: Some("tenant-a".to_string()),
                ..Default::default()
            },
            spec: IdentityInstanceSpec {
                agents: None,
                restore: None,
                iam: None,
                organisation_id: "org".to_string(),
                provider: IdentityProvider::Ferriskey,
                version: "1.0.0".to_string(),
                hostname: "auth.acme.test".to_string(),
                replicas: 1,
                stopped: false,
                database: DatabaseConfig {
                    mode: DatabaseMode::ManagedCluster,
                    shared: None,
                    managed_cluster: ManagedClusterConfig {
                        instances: 1,
                        storage: ManagedClusterStorage {
                            size: "1Gi".to_string(),
                            storage_class: None,
                        },
                        resources: ResourceRequirements {
                            requests: None,
                            limits: None,
                        },
                    },
                },
                ferriskey: None,
                ingress: None,
                allowed_cidrs: None,
                backup: None,
                webhooks: Vec::new(),
                webhooks_revision: 0,
                webhook_requests: Vec::new(),
                smtp: Vec::new(),
                portal_themes: Vec::new(),
                iam_requests: requests,
            },
            status: Some(IdentityInstanceStatus {
                phase: Some(Phase::Running),
                ready: true,
                ..Default::default()
            }),
        }
    }

    #[derive(Default)]
    struct FakeFerriskey {
        calls: Mutex<Vec<(String, IamOperation)>>,
        answers: Mutex<Vec<Result<IamResult, IamError>>>,
    }

    impl FakeFerriskey {
        fn answering(answer: Result<IamResult, IamError>) -> Self {
            Self {
                calls: Mutex::default(),
                answers: Mutex::new(vec![answer]),
            }
        }

        fn calls(&self) -> Vec<(String, IamOperation)> {
            self.calls.lock().expect("not poisoned").clone()
        }
    }

    impl FerriskeyIamPort for FakeFerriskey {
        async fn execute(
            &self,
            _instance: &IdentityInstance,
            realm: &str,
            operation: &IamOperation,
        ) -> Result<IamResult, IamError> {
            self.calls
                .lock()
                .expect("not poisoned")
                .push((realm.to_string(), operation.clone()));
            let mut answers = self.answers.lock().expect("not poisoned");
            match answers.len() {
                0 => Ok(page(1)),
                1 => answers[0].clone(),
                _ => answers.remove(0),
            }
        }
    }

    #[derive(Default)]
    struct FakeResults {
        stored: Mutex<BTreeMap<String, String>>,
        forgotten: Mutex<Vec<String>>,
        failure: Mutex<Option<IamError>>,
    }

    impl IamResultsPort for FakeResults {
        async fn put(
            &self,
            _instance: &IdentityInstance,
            request_id: &str,
            envelope: &str,
        ) -> Result<(), IamError> {
            if let Some(error) = self.failure.lock().expect("not poisoned").clone() {
                return Err(error);
            }
            self.stored
                .lock()
                .expect("not poisoned")
                .insert(request_id.to_string(), envelope.to_string());
            Ok(())
        }

        async fn forget(
            &self,
            _instance: &IdentityInstance,
            request_ids: &[&str],
        ) -> Result<(), IamError> {
            self.forgotten
                .lock()
                .expect("not poisoned")
                .extend(request_ids.iter().map(ToString::to_string));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeStatus {
        recorded: Mutex<Vec<IamRequestStatus>>,
        patches: Mutex<usize>,
        read_fails: Mutex<bool>,
        patch_fails: Mutex<bool>,
    }

    impl IamStatusRepository for FakeStatus {
        async fn recorded(
            &self,
            _instance: &IdentityInstance,
        ) -> Result<Vec<IamRequestStatus>, OperatorError> {
            if *self.read_fails.lock().expect("not poisoned") {
                return Err(OperatorError::Kube {
                    message: "down".to_string(),
                });
            }
            Ok(self.recorded.lock().expect("not poisoned").clone())
        }

        async fn patch_requests(
            &self,
            _instance: &IdentityInstance,
            requests: &[IamRequestStatus],
        ) -> Result<(), OperatorError> {
            if *self.patch_fails.lock().expect("not poisoned") {
                return Err(OperatorError::Kube {
                    message: "down".to_string(),
                });
            }
            *self.patches.lock().expect("not poisoned") += 1;
            *self.recorded.lock().expect("not poisoned") = requests.to_vec();
            Ok(())
        }
    }

    struct Harness {
        reconciler: IamReconciler<FakeStatus, FakeFerriskey, FakeResults>,
        status: Arc<FakeStatus>,
        ferriskey: Arc<FakeFerriskey>,
        results: Arc<FakeResults>,
    }

    fn harness(ferriskey: FakeFerriskey) -> Harness {
        let status = Arc::new(FakeStatus::default());
        let ferriskey = Arc::new(ferriskey);
        let results = Arc::new(FakeResults::default());
        Harness {
            reconciler: IamReconciler::new(
                Arc::clone(&status),
                Arc::clone(&ferriskey),
                Arc::clone(&results),
            ),
            status,
            ferriskey,
            results,
        }
    }

    fn page(count: usize) -> IamResult {
        IamResult::Applications(ApplicationPage {
            items: (0..count).map(|n| summary(&n.to_string())).collect(),
            next_page: None,
        })
    }

    fn recorded(harness: &Harness) -> Vec<IamRequestStatus> {
        harness
            .status
            .recorded
            .lock()
            .expect("not poisoned")
            .clone()
    }

    fn stored(harness: &Harness, id: &str) -> Option<Envelope> {
        harness
            .results
            .stored
            .lock()
            .expect("not poisoned")
            .get(id)
            .map(|text| serde_json::from_str(text).expect("an envelope"))
    }

    #[tokio::test]
    async fn a_request_is_run_sealed_to_its_key_and_only_the_envelope_is_stored() {
        let h = harness(FakeFerriskey::answering(Ok(page(2))));

        let outcome = h
            .reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert_eq!(
            recorded(&h),
            vec![IamRequestStatus {
                request_id: FIRST.to_string(),
                phase: IamRequestPhase::Done,
                message: None,
            }]
        );
        let envelope = stored(&h, FIRST).expect("stored under the request id");
        let plain = open(&private_key(), &RequestId::parse(FIRST).unwrap(), &envelope)
            .expect("opens for its request");
        let opened: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(opened["items"].as_array().unwrap().len(), 2);
        assert_eq!(opened["items"][0]["id"], json!("0"));
        let stored_text = h.results.stored.lock().unwrap()[FIRST].clone();
        assert!(!stored_text.contains("App 0"));
    }

    #[tokio::test]
    async fn the_call_carries_the_realm_and_the_named_operation() {
        let h = harness(FakeFerriskey::default());
        let mut asked = request(FIRST);
        asked.realm = "acme".to_string();
        asked.operation = json!({ "type": "applications.get", "id": APP }).to_string();

        h.reconciler
            .reconcile_at(&instance(vec![asked]), now())
            .await;

        let calls = h.ferriskey.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "acme");
        assert_eq!(
            calls[0].1,
            IamOperation::ApplicationsGet {
                id: uuid::Uuid::parse_str(APP).unwrap()
            }
        );
    }

    #[tokio::test]
    async fn spec_iam_10_a_redelivered_request_calls_ferriskey_once() {
        let h = harness(FakeFerriskey::default());
        let asked = instance(vec![request(FIRST)]);

        h.reconciler.reconcile_at(&asked, now()).await;
        h.reconciler.reconcile_at(&asked, now()).await;
        h.reconciler.reconcile_at(&asked, now()).await;

        assert_eq!(h.ferriskey.calls().len(), 1);
        assert_eq!(recorded(&h).len(), 1);
        assert_eq!(*h.status.patches.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_request_already_recorded_before_a_restart_is_not_run_again() {
        let h = harness(FakeFerriskey::default());
        h.status.recorded.lock().unwrap().push(IamRequestStatus {
            request_id: FIRST.to_string(),
            phase: IamRequestPhase::Done,
            message: None,
        });

        let outcome = h
            .reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(*h.status.patches.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn spec_iam_11_a_result_over_512_kib_ends_failed_and_is_never_cut() {
        let mut big = summary("1");
        big.name = "n".repeat(MAX_RESULT_BYTES);
        let h = harness(FakeFerriskey::answering(Ok(IamResult::Applications(
            ApplicationPage {
                items: vec![big],
                next_page: None,
            },
        ))));

        h.reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(
            recorded(&h),
            vec![IamRequestStatus {
                request_id: FIRST.to_string(),
                phase: IamRequestPhase::Failed,
                message: Some("result too large".to_string()),
            }]
        );
        assert!(stored(&h, FIRST).is_none());
    }

    #[tokio::test]
    async fn a_result_of_exactly_512_kib_is_still_sealed() {
        let empty = IamResult::Applications(ApplicationPage {
            items: vec![{
                let mut item = summary("1");
                item.name = String::new();
                item
            }],
            next_page: None,
        });
        let base = empty.to_json().unwrap().len();
        let mut exact = summary("1");
        exact.name = "n".repeat(MAX_RESULT_BYTES - base);
        let result = IamResult::Applications(ApplicationPage {
            items: vec![exact],
            next_page: None,
        });
        assert_eq!(result.to_json().unwrap().len(), MAX_RESULT_BYTES);
        let h = harness(FakeFerriskey::answering(Ok(result)));

        h.reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Done);
        assert!(stored(&h, FIRST).is_some());
    }

    #[tokio::test]
    async fn a_list_asking_for_more_than_fifty_is_clamped_before_ferriskey_sees_it() {
        let h = harness(FakeFerriskey::default());
        let mut asked = request(FIRST);
        asked.operation =
            json!({ "type": "applications.list", "page": 2, "limit": 500 }).to_string();

        h.reconciler
            .reconcile_at(&instance(vec![asked]), now())
            .await;

        let IamOperation::ApplicationsList { page, limit, .. } = h.ferriskey.calls()[0].1.clone()
        else {
            panic!("a list");
        };
        assert_eq!(limit, PageSize::clamped(50));
        assert_eq!(page.get(), 2);
    }

    #[tokio::test]
    async fn an_expired_entry_is_dropped_without_a_call() {
        let h = harness(FakeFerriskey::default());
        let mut late = request(FIRST);
        late.expires_at = "2026-10-24T11:59:59Z".to_string();

        let outcome = h
            .reconciler
            .reconcile_at(&instance(vec![late]), now())
            .await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(stored(&h, FIRST).is_none());
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Failed);
        assert_eq!(
            recorded(&h)[0].message.as_deref(),
            Some("the request expired before it was run")
        );
    }

    #[tokio::test]
    async fn an_expiry_that_is_not_a_date_fails_the_request() {
        let h = harness(FakeFerriskey::default());
        let mut broken = request(FIRST);
        broken.expires_at = "soon".to_string();

        h.reconciler
            .reconcile_at(&instance(vec![broken]), now())
            .await;

        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Failed);
    }

    #[tokio::test]
    async fn a_weak_or_garbled_sealing_key_fails_the_request_before_any_call() {
        let h = harness(FakeFerriskey::default());
        let mut small = request(FIRST);
        small.sealing_key = json!({
            "kty": "RSA", "alg": "RSA-OAEP-256", "e": "AQAB",
            "n": URL_SAFE_NO_PAD.encode([0xffu8; 128]),
        })
        .to_string();
        let mut garbled = request(SECOND);
        garbled.sealing_key = "not a key".to_string();

        h.reconciler
            .reconcile_at(&instance(vec![small, garbled]), now())
            .await;

        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(
            recorded(&h)[0].message.as_deref(),
            Some("the sealing key is under 2048 bits")
        );
        assert_eq!(
            recorded(&h)[1].message.as_deref(),
            Some("the sealing key is not well formed")
        );
    }

    #[tokio::test]
    async fn an_operation_that_does_not_exist_fails_the_request_before_any_call() {
        let h = harness(FakeFerriskey::default());
        let mut unknown = request(FIRST);
        unknown.operation = json!({ "type": "applications.delete", "id": APP }).to_string();

        h.reconciler
            .reconcile_at(&instance(vec![unknown]), now())
            .await;

        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Failed);
    }

    #[tokio::test]
    async fn a_request_id_that_is_not_a_uuid_fails_the_request() {
        let h = harness(FakeFerriskey::default());

        h.reconciler
            .reconcile_at(&instance(vec![request("../etc")]), now())
            .await;

        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(recorded(&h)[0].request_id, "../etc");
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Failed);
    }

    #[tokio::test]
    async fn a_ferriskey_that_is_down_is_tried_again_and_nothing_is_recorded() {
        let h = harness(FakeFerriskey::answering(Err(IamError::Retry {
            message: "FerrisKey is unreachable".to_string(),
        })));
        let asked = instance(vec![request(FIRST)]);

        let outcome = h.reconciler.reconcile_at(&asked, now()).await;

        assert_eq!(outcome, IamOutcome::Retry);
        assert!(recorded(&h).is_empty());
        assert_eq!(*h.status.patches.lock().unwrap(), 0);

        *h.ferriskey.answers.lock().unwrap() = vec![Ok(page(1))];
        let outcome = h.reconciler.reconcile_at(&asked, now()).await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Done);
    }

    #[tokio::test]
    async fn a_retry_ends_once_the_request_expires() {
        let h = harness(FakeFerriskey::answering(Err(IamError::Retry {
            message: "FerrisKey is unreachable".to_string(),
        })));
        let asked = instance(vec![request(FIRST)]);
        h.reconciler.reconcile_at(&asked, now()).await;

        let later = now() + chrono::Duration::minutes(6);
        let outcome = h.reconciler.reconcile_at(&asked, later).await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert_eq!(h.ferriskey.calls().len(), 1);
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Failed);
    }

    #[tokio::test]
    async fn a_refusal_from_ferriskey_is_recorded_with_its_message() {
        let h = harness(FakeFerriskey::answering(Err(IamError::rejected(
            "FerrisKey answered 404",
        ))));

        h.reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(
            recorded(&h)[0].message.as_deref(),
            Some("FerrisKey answered 404")
        );
        assert_eq!(recorded(&h)[0].phase, IamRequestPhase::Failed);
        assert!(stored(&h, FIRST).is_none());
    }

    #[tokio::test]
    async fn a_store_that_cannot_be_written_is_retried_and_one_that_refuses_fails_the_request() {
        let h = harness(FakeFerriskey::default());
        *h.results.failure.lock().unwrap() = Some(IamError::Retry {
            message: "the cluster is busy".to_string(),
        });
        let asked = instance(vec![request(FIRST)]);

        assert_eq!(
            h.reconciler.reconcile_at(&asked, now()).await,
            IamOutcome::Retry
        );
        assert!(recorded(&h).is_empty());

        *h.results.failure.lock().unwrap() = Some(IamError::rejected("it does not fit"));
        h.reconciler.reconcile_at(&asked, now()).await;

        assert_eq!(recorded(&h)[0].message.as_deref(), Some("it does not fit"));
    }

    #[tokio::test]
    async fn requests_are_independent_and_run_in_order() {
        let h = harness(FakeFerriskey {
            calls: Mutex::default(),
            answers: Mutex::new(vec![Err(IamError::rejected("no")), Ok(page(1))]),
        });

        h.reconciler
            .reconcile_at(&instance(vec![request(FIRST), request(SECOND)]), now())
            .await;

        let phases: Vec<_> = recorded(&h).iter().map(|r| r.phase).collect();
        assert_eq!(phases, [IamRequestPhase::Failed, IamRequestPhase::Done]);
        assert!(stored(&h, SECOND).is_some());
        assert!(stored(&h, FIRST).is_none());
    }

    #[tokio::test]
    async fn a_detail_is_sealed_as_well() {
        let h = harness(FakeFerriskey::answering(Ok(IamResult::Application(
            Box::new(detail(APP)),
        ))));

        h.reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        let envelope = stored(&h, FIRST).expect("stored");
        let plain = open(&private_key(), &RequestId::parse(FIRST).unwrap(), &envelope).unwrap();
        let opened: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(opened["id"], json!(APP));
        assert!(opened.get("secret").is_none());
    }

    #[tokio::test]
    async fn nothing_asked_means_nothing_read_and_nothing_called() {
        let h = harness(FakeFerriskey::default());
        *h.status.read_fails.lock().unwrap() = true;

        let outcome = h
            .reconciler
            .reconcile_at(&instance(Vec::new()), now())
            .await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
    }

    #[tokio::test]
    async fn an_instance_that_is_not_ready_waits() {
        let h = harness(FakeFerriskey::default());
        let mut waiting = instance(vec![request(FIRST)]);
        waiting.status = Some(IdentityInstanceStatus {
            phase: Some(Phase::Deploying),
            ready: false,
            ..Default::default()
        });

        let outcome = h.reconciler.reconcile_at(&waiting, now()).await;

        assert_eq!(outcome, IamOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
    }

    #[tokio::test]
    async fn a_keycloak_instance_is_left_alone() {
        let h = harness(FakeFerriskey::default());
        let mut other = instance(vec![request(FIRST)]);
        other.spec.provider = IdentityProvider::Keycloak;

        h.reconciler.reconcile_at(&other, now()).await;

        assert!(h.ferriskey.calls().is_empty());
    }

    #[tokio::test]
    async fn a_status_that_cannot_be_read_stops_the_pass_so_nothing_runs_twice() {
        let h = harness(FakeFerriskey::default());
        *h.status.read_fails.lock().unwrap() = true;

        let outcome = h
            .reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(outcome, IamOutcome::Retry);
        assert!(h.ferriskey.calls().is_empty());
    }

    #[tokio::test]
    async fn a_status_that_cannot_be_written_asks_for_another_pass() {
        let h = harness(FakeFerriskey::default());
        *h.status.patch_fails.lock().unwrap() = true;

        let outcome = h
            .reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert_eq!(outcome, IamOutcome::Retry);
    }

    #[tokio::test]
    async fn the_stored_result_of_an_expired_request_is_dropped() {
        let h = harness(FakeFerriskey::default());
        h.status.recorded.lock().unwrap().push(IamRequestStatus {
            request_id: FIRST.to_string(),
            phase: IamRequestPhase::Done,
            message: None,
        });
        let asked = instance(vec![request(FIRST), request(SECOND)]);
        let later = now() + chrono::Duration::minutes(6);

        h.reconciler.reconcile_at(&asked, later).await;

        assert_eq!(*h.results.forgotten.lock().unwrap(), [FIRST.to_string()]);
    }

    #[tokio::test]
    async fn a_result_that_has_not_expired_is_kept() {
        let h = harness(FakeFerriskey::default());
        h.status.recorded.lock().unwrap().push(IamRequestStatus {
            request_id: FIRST.to_string(),
            phase: IamRequestPhase::Done,
            message: None,
        });

        h.reconciler
            .reconcile_at(&instance(vec![request(FIRST)]), now())
            .await;

        assert!(h.results.forgotten.lock().unwrap().is_empty());
    }

    #[test]
    fn recorded_requests_are_capped_but_the_ones_still_asked_for_stay() {
        let mut all: Vec<IamRequestStatus> = (0..=MAX_RECORDED_REQUESTS)
            .map(|n| IamRequestStatus {
                request_id: format!("r{n}"),
                phase: IamRequestPhase::Done,
                message: None,
            })
            .collect();
        all.push(IamRequestStatus {
            request_id: "kept".to_string(),
            phase: IamRequestPhase::Done,
            message: None,
        });
        let mut asked = request("kept");
        asked.request_id = "r0".to_string();

        trim_recorded(&mut all, &[asked]);

        assert_eq!(all.len(), MAX_RECORDED_REQUESTS);
        assert!(all.iter().any(|done| done.request_id == "r0"));
        assert!(!all.iter().any(|done| done.request_id == "r1"));
    }
}
