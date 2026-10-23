use std::sync::Arc;

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, IdentityProvider, PortalThemePhase, PortalThemeSpec, PortalThemeStatus,
};

use super::PortalThemeError;
use super::plan::{Step, decide};
use super::ports::{
    FerriskeyPortalThemePort, IdentityInstancePortalThemes, PortalThemeOutcome,
    PortalThemeStatusRepository,
};

pub struct PortalThemeReconciler<R, F> {
    repository: Arc<R>,
    ferriskey: Arc<F>,
}

impl<R, F> PortalThemeReconciler<R, F> {
    pub fn new(repository: Arc<R>, ferriskey: Arc<F>) -> Self {
        Self {
            repository,
            ferriskey,
        }
    }
}

fn report(
    entry: &PortalThemeSpec,
    phase: PortalThemePhase,
    message: Option<String>,
) -> PortalThemeStatus {
    PortalThemeStatus {
        realm: entry.realm.clone(),
        revision: entry.revision,
        phase,
        message,
    }
}

fn failed(entry: &PortalThemeSpec, error: PortalThemeError) -> PortalThemeStatus {
    let phase = if error.is_retry() {
        PortalThemePhase::Pending
    } else {
        PortalThemePhase::Failed
    };
    report(entry, phase, Some(error.message().to_string()))
}

impl<R, F> IdentityInstancePortalThemes for PortalThemeReconciler<R, F>
where
    R: PortalThemeStatusRepository,
    F: FerriskeyPortalThemePort,
{
    async fn reconcile(&self, instance: &IdentityInstance) -> PortalThemeOutcome {
        if instance.spec.provider != IdentityProvider::Ferriskey
            || instance.spec.stopped
            || !instance.is_ready()
        {
            return PortalThemeOutcome::Settled;
        }

        let wanted = instance.spec.portal_themes.as_slice();
        let previous = instance
            .status
            .as_ref()
            .map_or(&[][..], |status| status.portal_themes.as_slice());
        if wanted.is_empty() && previous.is_empty() {
            return PortalThemeOutcome::Settled;
        }

        let mut retry = false;
        let mut statuses = Vec::with_capacity(wanted.len());
        for entry in wanted {
            let before = previous.iter().find(|status| status.realm == entry.realm);
            let status = self.settle(instance, entry, before).await;
            retry |= status.phase == PortalThemePhase::Pending;
            statuses.push(status);
        }

        if statuses != previous
            && let Err(error) = self
                .repository
                .patch_portal_theme_status(instance, &statuses)
                .await
        {
            tracing::warn!(error = %error, "could not report the portal theme status");
            retry = true;
        }

        if retry {
            PortalThemeOutcome::Retry
        } else {
            PortalThemeOutcome::Settled
        }
    }
}

impl<R, F> PortalThemeReconciler<R, F>
where
    F: FerriskeyPortalThemePort,
{
    async fn settle(
        &self,
        instance: &IdentityInstance,
        entry: &PortalThemeSpec,
        before: Option<&PortalThemeStatus>,
    ) -> PortalThemeStatus {
        match decide(entry, before) {
            Step::Keep(status) => status.clone(),
            Step::Apply(config) => {
                match self
                    .ferriskey
                    .put_theme(instance, &entry.realm, config)
                    .await
                {
                    Ok(()) => report(entry, PortalThemePhase::Applied, None),
                    Err(error) => failed(entry, error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::domain::OperatorError;

    #[derive(Default)]
    struct FerriskeyState {
        calls: Vec<(String, Value)>,
        failures: BTreeMap<String, PortalThemeError>,
    }

    #[derive(Default)]
    struct FakeFerriskey {
        state: Mutex<FerriskeyState>,
    }

    impl FakeFerriskey {
        fn failing(realm: &str, error: PortalThemeError) -> Self {
            let fake = Self::default();
            fake.state
                .lock()
                .unwrap()
                .failures
                .insert(realm.to_string(), error);
            fake
        }

        fn calls(&self) -> Vec<(String, Value)> {
            self.state.lock().unwrap().calls.clone()
        }
    }

    impl FerriskeyPortalThemePort for FakeFerriskey {
        async fn put_theme(
            &self,
            _: &IdentityInstance,
            realm: &str,
            config: &Value,
        ) -> Result<(), PortalThemeError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push((realm.to_string(), config.clone()));
            state.failures.get(realm).cloned().map_or(Ok(()), Err)
        }
    }

    #[derive(Default)]
    struct FakeRepository {
        patches: Mutex<Vec<Vec<PortalThemeStatus>>>,
        broken: bool,
    }

    impl FakeRepository {
        fn patches(&self) -> Vec<Vec<PortalThemeStatus>> {
            self.patches.lock().unwrap().clone()
        }
    }

    impl PortalThemeStatusRepository for FakeRepository {
        async fn patch_portal_theme_status(
            &self,
            _: &IdentityInstance,
            statuses: &[PortalThemeStatus],
        ) -> Result<(), OperatorError> {
            if self.broken {
                return Err(OperatorError::Kube {
                    message: "no".to_string(),
                });
            }
            self.patches.lock().unwrap().push(statuses.to_vec());
            Ok(())
        }
    }

    struct Harness {
        reconciler: PortalThemeReconciler<FakeRepository, FakeFerriskey>,
        repository: Arc<FakeRepository>,
        ferriskey: Arc<FakeFerriskey>,
    }

    fn harness(ferriskey: FakeFerriskey) -> Harness {
        with_repository(ferriskey, FakeRepository::default())
    }

    fn with_repository(ferriskey: FakeFerriskey, repository: FakeRepository) -> Harness {
        let repository = Arc::new(repository);
        let ferriskey = Arc::new(ferriskey);
        Harness {
            reconciler: PortalThemeReconciler::new(Arc::clone(&repository), Arc::clone(&ferriskey)),
            repository,
            ferriskey,
        }
    }

    fn config() -> Value {
        json!({ "colors": { "primaryButton": "#112233" }, "fonts": { "baseSize": 16 } })
    }

    fn entry(realm: &str, revision: u64) -> PortalThemeSpec {
        PortalThemeSpec {
            realm: realm.to_string(),
            revision,
            config: config(),
        }
    }

    fn status(realm: &str, revision: u64, phase: PortalThemePhase) -> PortalThemeStatus {
        PortalThemeStatus {
            realm: realm.to_string(),
            revision,
            phase,
            message: None,
        }
    }

    fn instance(
        provider: &str,
        ready: bool,
        wanted: &[PortalThemeSpec],
        reported: &[PortalThemeStatus],
    ) -> IdentityInstance {
        let value = json!({
            "apiVersion": "autharie.fr/v1alpha",
            "kind": "IdentityInstance",
            "metadata": { "name": "acme", "namespace": "tenant-a" },
            "spec": {
                "organisationId": "org",
                "provider": provider,
                "version": "1.0.0",
                "hostname": "auth.acme.test",
                "database": {
                    "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} }
                },
                "portalThemes": wanted,
            },
            "status": { "phase": "Running", "ready": ready, "portalThemes": reported },
        });
        serde_json::from_value(value).unwrap()
    }

    fn running(wanted: &[PortalThemeSpec], reported: &[PortalThemeStatus]) -> IdentityInstance {
        instance("ferriskey", true, wanted, reported)
    }

    #[tokio::test]
    async fn a_new_entry_is_put_verbatim_and_reported_applied() {
        let h = harness(FakeFerriskey::default());

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Settled);
        assert_eq!(h.ferriskey.calls(), vec![("acme".to_string(), config())]);
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 1, PortalThemePhase::Applied)]]
        );
    }

    #[tokio::test]
    async fn empty_tokens_are_put_as_such() {
        let h = harness(FakeFerriskey::default());
        let wanted = PortalThemeSpec {
            config: json!({}),
            ..entry("acme", 2)
        };

        h.reconciler.reconcile(&running(&[wanted], &[])).await;

        assert_eq!(h.ferriskey.calls(), vec![("acme".to_string(), json!({}))]);
    }

    #[tokio::test]
    async fn a_temporary_failure_is_pending_with_its_message_and_asks_for_a_retry() {
        let h = harness(FakeFerriskey::failing(
            "acme",
            PortalThemeError::Retry {
                message: "FerrisKey is unreachable".to_string(),
            },
        ));

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Retry);
        assert_eq!(
            h.repository.patches(),
            vec![vec![PortalThemeStatus {
                message: Some("FerrisKey is unreachable".to_string()),
                ..status("acme", 1, PortalThemePhase::Pending)
            }]]
        );
    }

    #[tokio::test]
    async fn a_pending_entry_is_put_again_on_the_next_pass() {
        let h = harness(FakeFerriskey::default());
        let reported = [PortalThemeStatus {
            message: Some("FerrisKey is unreachable".to_string()),
            ..status("acme", 1, PortalThemePhase::Pending)
        }];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &reported))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Settled);
        assert_eq!(h.ferriskey.calls().len(), 1);
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 1, PortalThemePhase::Applied)]]
        );
    }

    #[tokio::test]
    async fn a_refusal_is_failed_with_ferriskeys_message_and_settles() {
        let h = harness(FakeFerriskey::failing(
            "acme",
            PortalThemeError::Rejected {
                message: "FerrisKey answered 400: bad colour".to_string(),
            },
        ));

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Settled);
        assert_eq!(
            h.repository.patches(),
            vec![vec![PortalThemeStatus {
                message: Some("FerrisKey answered 400: bad colour".to_string()),
                ..status("acme", 1, PortalThemePhase::Failed)
            }]]
        );
    }

    #[tokio::test]
    async fn a_failed_entry_is_not_tried_again_until_its_revision_moves() {
        let h = harness(FakeFerriskey::default());
        let reported = [status("acme", 1, PortalThemePhase::Failed)];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &reported))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());

        h.reconciler
            .reconcile(&running(&[entry("acme", 2)], &reported))
            .await;

        assert_eq!(h.ferriskey.calls().len(), 1);
    }

    #[tokio::test]
    async fn an_applied_entry_costs_no_call_and_no_write() {
        let h = harness(FakeFerriskey::default());
        let reported = [status("acme", 3, PortalThemePhase::Applied)];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 3)], &reported))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn each_realm_is_gated_on_its_own_revision() {
        let h = harness(FakeFerriskey::default());
        let reported = [
            status("acme", 3, PortalThemePhase::Applied),
            status("beta", 1, PortalThemePhase::Applied),
        ];

        h.reconciler
            .reconcile(&running(&[entry("acme", 3), entry("beta", 2)], &reported))
            .await;

        assert_eq!(h.ferriskey.calls(), vec![("beta".to_string(), config())]);
        assert_eq!(
            h.repository.patches(),
            vec![vec![
                status("acme", 3, PortalThemePhase::Applied),
                status("beta", 2, PortalThemePhase::Applied),
            ]]
        );
    }

    #[tokio::test]
    async fn one_realm_failing_does_not_hold_back_the_others() {
        let h = harness(FakeFerriskey::failing(
            "acme",
            PortalThemeError::Retry {
                message: "FerrisKey answered 503".to_string(),
            },
        ));

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1), entry("beta", 1)], &[]))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Retry);
        assert_eq!(h.ferriskey.calls().len(), 2);
        let patches = h.repository.patches();
        assert_eq!(patches[0][0].phase, PortalThemePhase::Pending);
        assert_eq!(patches[0][1].phase, PortalThemePhase::Applied);
    }

    #[tokio::test]
    async fn status_entries_of_realms_that_left_the_spec_are_dropped() {
        let h = harness(FakeFerriskey::default());
        let reported = [
            status("acme", 1, PortalThemePhase::Applied),
            status("gone", 4, PortalThemePhase::Applied),
        ];

        h.reconciler
            .reconcile(&running(&[entry("acme", 1)], &reported))
            .await;

        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 1, PortalThemePhase::Applied)]]
        );
    }

    #[tokio::test]
    async fn an_emptied_spec_clears_what_was_reported() {
        let h = harness(FakeFerriskey::default());
        let reported = [status("acme", 1, PortalThemePhase::Applied)];

        h.reconciler.reconcile(&running(&[], &reported)).await;

        assert_eq!(
            h.repository.patches(),
            vec![Vec::<PortalThemeStatus>::new()]
        );
        assert!(h.ferriskey.calls().is_empty());
    }

    #[tokio::test]
    async fn an_instance_without_portal_themes_is_left_alone_entirely() {
        let h = harness(FakeFerriskey::default());

        let outcome = h.reconciler.reconcile(&running(&[], &[])).await;

        assert_eq!(outcome, PortalThemeOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn only_a_ready_running_ferriskey_instance_is_reconciled() {
        let h = harness(FakeFerriskey::default());
        let wanted = [entry("acme", 1)];
        let mut stopped = running(&wanted, &[]);
        stopped.spec.stopped = true;

        for candidate in [
            instance("keycloak", true, &wanted, &[]),
            instance("ferriskey", false, &wanted, &[]),
            stopped,
        ] {
            assert_eq!(
                h.reconciler.reconcile(&candidate).await,
                PortalThemeOutcome::Settled
            );
        }

        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn a_status_that_could_not_be_written_asks_for_a_retry() {
        let h = with_repository(
            FakeFerriskey::default(),
            FakeRepository {
                broken: true,
                ..Default::default()
            },
        );

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, PortalThemeOutcome::Retry);
    }

    #[tokio::test]
    async fn the_status_has_one_entry_per_spec_entry_in_spec_order() {
        let h = harness(FakeFerriskey::default());

        h.reconciler
            .reconcile(&running(
                &[entry("z", 1), entry("b", 1), entry("a", 1)],
                &[],
            ))
            .await;

        let patches = h.repository.patches();
        let realms: Vec<_> = patches[0]
            .iter()
            .map(|status| status.realm.as_str())
            .collect();
        assert_eq!(realms, vec!["z", "b", "a"]);
    }
}
