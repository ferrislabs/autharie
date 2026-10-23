use std::collections::BTreeSet;
use std::sync::Arc;

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, IdentityProvider, SmtpPhase, SmtpSpec, SmtpStatus,
};

use super::SmtpError;
use super::plan::{Passwords, Step, decide};
use super::ports::{
    FerriskeySmtpPort, IdentityInstanceSmtp, SmtpOutcome, SmtpSecrets, SmtpSecretsPort,
    SmtpStatusRepository,
};

pub struct SmtpReconciler<R, F, K> {
    repository: Arc<R>,
    ferriskey: Arc<F>,
    secrets: Arc<K>,
}

impl<R, F, K> SmtpReconciler<R, F, K> {
    pub fn new(repository: Arc<R>, ferriskey: Arc<F>, secrets: Arc<K>) -> Self {
        Self {
            repository,
            ferriskey,
            secrets,
        }
    }
}

fn report(entry: &SmtpSpec, phase: SmtpPhase, message: Option<String>) -> SmtpStatus {
    SmtpStatus {
        realm: entry.realm.clone(),
        revision: entry.revision,
        phase,
        message,
    }
}

fn failed(entry: &SmtpSpec, error: SmtpError) -> SmtpStatus {
    let phase = if error.is_retry() {
        SmtpPhase::Pending
    } else {
        SmtpPhase::Failed
    };
    report(entry, phase, Some(error.message().to_string()))
}

impl<R, F, K> IdentityInstanceSmtp for SmtpReconciler<R, F, K>
where
    R: SmtpStatusRepository,
    F: FerriskeySmtpPort,
    K: SmtpSecretsPort,
{
    async fn reconcile(&self, instance: &IdentityInstance) -> SmtpOutcome {
        if instance.spec.provider != IdentityProvider::Ferriskey
            || instance.spec.stopped
            || !instance.is_ready()
        {
            return SmtpOutcome::Settled;
        }

        let wanted = instance.spec.smtp.as_slice();
        let previous = instance
            .status
            .as_ref()
            .map_or(&[][..], |status| status.smtp.as_slice());
        if wanted.is_empty() && previous.is_empty() {
            return SmtpOutcome::Settled;
        }

        let secrets = match self.secrets.load(instance).await {
            Ok(secrets) => secrets,
            Err(error) => {
                tracing::warn!(error = %error.message(), "could not read the smtp secrets");
                return SmtpOutcome::Retry;
            }
        };

        let mut retry = false;
        let mut statuses = Vec::with_capacity(wanted.len());
        for entry in wanted {
            let before = previous.iter().find(|status| status.realm == entry.realm);
            let status = self.settle(instance, entry, before, &secrets).await;
            retry |= status.phase == SmtpPhase::Pending;
            statuses.push(status);
        }

        self.forget_stale(instance, &secrets, wanted, &statuses)
            .await;

        if statuses != previous
            && let Err(error) = self.repository.patch_smtp_status(instance, &statuses).await
        {
            tracing::warn!(error = %error, "could not report the smtp status");
            retry = true;
        }

        if retry {
            SmtpOutcome::Retry
        } else {
            SmtpOutcome::Settled
        }
    }
}

impl<R, F, K> SmtpReconciler<R, F, K>
where
    F: FerriskeySmtpPort,
    K: SmtpSecretsPort,
{
    async fn settle(
        &self,
        instance: &IdentityInstance,
        entry: &SmtpSpec,
        before: Option<&SmtpStatus>,
        secrets: &SmtpSecrets,
    ) -> SmtpStatus {
        let passwords = Passwords {
            stored: secrets.stored.get(&entry.realm).map(String::as_str),
            applied: secrets.applied.get(&entry.realm).map(String::as_str),
        };
        match decide(entry, before, &passwords) {
            Step::Keep(status) => status.clone(),
            Step::Fail(message) => report(entry, SmtpPhase::Failed, Some(message)),
            Step::Remove => match self.ferriskey.delete_config(instance, &entry.realm).await {
                Ok(()) => report(entry, SmtpPhase::Applied, None),
                Err(error) => failed(entry, error),
            },
            Step::Apply(write) => {
                if let Err(error) = self
                    .ferriskey
                    .put_config(instance, &entry.realm, &write)
                    .await
                {
                    return failed(entry, error);
                }
                if passwords.applied == Some(write.password) {
                    return report(entry, SmtpPhase::Applied, None);
                }
                match self
                    .secrets
                    .put_applied(instance, &entry.realm, write.password)
                    .await
                {
                    Ok(()) => report(entry, SmtpPhase::Applied, None),
                    Err(error) => {
                        tracing::warn!(realm = %entry.realm, error = %error.message(), "could not keep the applied smtp password");
                        report(
                            entry,
                            SmtpPhase::Pending,
                            Some("applied, but its password could not be kept".to_string()),
                        )
                    }
                }
            }
        }
    }

    /// Passwords kept for a realm that no longer has a provider, or no entry.
    async fn forget_stale(
        &self,
        instance: &IdentityInstance,
        secrets: &SmtpSecrets,
        wanted: &[SmtpSpec],
        statuses: &[SmtpStatus],
    ) {
        let listed: BTreeSet<&str> = wanted.iter().map(|entry| entry.realm.as_str()).collect();
        let removed: BTreeSet<&str> = wanted
            .iter()
            .zip(statuses)
            .filter(|(entry, status)| entry.remove && status.phase == SmtpPhase::Applied)
            .map(|(entry, _)| entry.realm.as_str())
            .collect();
        let stale: Vec<&str> = secrets
            .applied
            .keys()
            .map(String::as_str)
            .filter(|realm| !listed.contains(realm) || removed.contains(realm))
            .collect();
        if stale.is_empty() {
            return;
        }
        if let Err(error) = self.secrets.forget_applied(instance, &stale).await {
            tracing::warn!(error = %error.message(), "could not drop the smtp passwords of removed realms");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use autharie_crds::v1alpha::identity_instance::SmtpEncryption;
    use serde_json::json;

    use super::*;
    use crate::domain::OperatorError;
    use crate::domain::smtp::SmtpWrite;

    #[derive(Default)]
    struct FerriskeyState {
        calls: Vec<String>,
        writes: Vec<(String, String, u16, String)>,
        failures: BTreeMap<String, SmtpError>,
    }

    #[derive(Default)]
    struct FakeFerriskey {
        state: Mutex<FerriskeyState>,
    }

    impl FakeFerriskey {
        fn failing(call: &str, error: SmtpError) -> Self {
            let fake = Self::default();
            fake.state
                .lock()
                .unwrap()
                .failures
                .insert(call.to_string(), error);
            fake
        }

        fn calls(&self) -> Vec<String> {
            self.state.lock().unwrap().calls.clone()
        }

        fn writes(&self) -> Vec<(String, String, u16, String)> {
            self.state.lock().unwrap().writes.clone()
        }

        fn answer(&self, call: String) -> Result<(), SmtpError> {
            let mut state = self.state.lock().unwrap();
            let failure = state.failures.get(&call).cloned();
            state.calls.push(call);
            failure.map_or(Ok(()), Err)
        }
    }

    impl FerriskeySmtpPort for FakeFerriskey {
        async fn put_config(
            &self,
            _: &IdentityInstance,
            realm: &str,
            write: &SmtpWrite<'_>,
        ) -> Result<(), SmtpError> {
            self.state.lock().unwrap().writes.push((
                realm.to_string(),
                write.password.to_string(),
                write.port,
                write.host.to_string(),
            ));
            self.answer(format!("put {realm}"))
        }

        async fn delete_config(&self, _: &IdentityInstance, realm: &str) -> Result<(), SmtpError> {
            self.answer(format!("delete {realm}"))
        }
    }

    #[derive(Default)]
    struct SecretsState {
        stored: BTreeMap<String, String>,
        applied: BTreeMap<String, String>,
        unreadable: bool,
        unwritable: bool,
    }

    #[derive(Default)]
    struct FakeSecrets {
        state: Mutex<SecretsState>,
    }

    impl FakeSecrets {
        fn with(stored: &[(&str, &str)], applied: &[(&str, &str)]) -> Self {
            let fake = Self::default();
            {
                let mut state = fake.state.lock().unwrap();
                let owned = |pairs: &[(&str, &str)]| {
                    pairs
                        .iter()
                        .map(|(realm, password)| (realm.to_string(), password.to_string()))
                        .collect()
                };
                state.stored = owned(stored);
                state.applied = owned(applied);
            }
            fake
        }

        fn applied(&self) -> BTreeMap<String, String> {
            self.state.lock().unwrap().applied.clone()
        }
    }

    impl SmtpSecretsPort for FakeSecrets {
        async fn load(&self, _: &IdentityInstance) -> Result<SmtpSecrets, SmtpError> {
            let state = self.state.lock().unwrap();
            if state.unreadable {
                return Err(SmtpError::Retry {
                    message: "cluster down".to_string(),
                });
            }
            Ok(SmtpSecrets {
                stored: state.stored.clone(),
                applied: state.applied.clone(),
            })
        }

        async fn put_applied(
            &self,
            _: &IdentityInstance,
            realm: &str,
            password: &str,
        ) -> Result<(), SmtpError> {
            let mut state = self.state.lock().unwrap();
            if state.unwritable {
                return Err(SmtpError::Retry {
                    message: "cluster down".to_string(),
                });
            }
            state
                .applied
                .insert(realm.to_string(), password.to_string());
            Ok(())
        }

        async fn forget_applied(
            &self,
            _: &IdentityInstance,
            realms: &[&str],
        ) -> Result<(), SmtpError> {
            let mut state = self.state.lock().unwrap();
            for realm in realms {
                state.applied.remove(*realm);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeRepository {
        patches: Mutex<Vec<Vec<SmtpStatus>>>,
        broken: bool,
    }

    impl FakeRepository {
        fn patches(&self) -> Vec<Vec<SmtpStatus>> {
            self.patches.lock().unwrap().clone()
        }
    }

    impl SmtpStatusRepository for FakeRepository {
        async fn patch_smtp_status(
            &self,
            _: &IdentityInstance,
            statuses: &[SmtpStatus],
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
        reconciler: SmtpReconciler<FakeRepository, FakeFerriskey, FakeSecrets>,
        repository: Arc<FakeRepository>,
        ferriskey: Arc<FakeFerriskey>,
        secrets: Arc<FakeSecrets>,
    }

    fn harness(ferriskey: FakeFerriskey, secrets: FakeSecrets) -> Harness {
        with_repository(ferriskey, secrets, FakeRepository::default())
    }

    fn with_repository(
        ferriskey: FakeFerriskey,
        secrets: FakeSecrets,
        repository: FakeRepository,
    ) -> Harness {
        let repository = Arc::new(repository);
        let ferriskey = Arc::new(ferriskey);
        let secrets = Arc::new(secrets);
        Harness {
            reconciler: SmtpReconciler::new(
                Arc::clone(&repository),
                Arc::clone(&ferriskey),
                Arc::clone(&secrets),
            ),
            repository,
            ferriskey,
            secrets,
        }
    }

    fn entry(realm: &str, revision: u64) -> SmtpSpec {
        SmtpSpec {
            realm: realm.to_string(),
            revision,
            remove: false,
            host: Some("smtp.example.com".to_string()),
            port: Some(587),
            encryption: Some(SmtpEncryption::Starttls),
            username: Some("mailer".to_string()),
            from_email: Some("no-reply@example.com".to_string()),
            from_name: Some("Acme".to_string()),
        }
    }

    fn tombstone(realm: &str, revision: u64) -> SmtpSpec {
        SmtpSpec {
            realm: realm.to_string(),
            revision,
            remove: true,
            host: None,
            port: None,
            encryption: None,
            username: None,
            from_email: None,
            from_name: None,
        }
    }

    fn status(realm: &str, revision: u64, phase: SmtpPhase) -> SmtpStatus {
        SmtpStatus {
            realm: realm.to_string(),
            revision,
            phase,
            message: None,
        }
    }

    fn instance(
        provider: &str,
        ready: bool,
        wanted: &[SmtpSpec],
        reported: &[SmtpStatus],
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
                "smtp": wanted,
            },
            "status": { "phase": "Running", "ready": ready, "smtp": reported },
        });
        serde_json::from_value(value).unwrap()
    }

    fn running(wanted: &[SmtpSpec], reported: &[SmtpStatus]) -> IdentityInstance {
        instance("ferriskey", true, wanted, reported)
    }

    #[tokio::test]
    async fn a_new_entry_is_put_and_reported_applied() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert_eq!(h.ferriskey.calls(), vec!["put acme"]);
        assert_eq!(
            h.ferriskey.writes(),
            vec![(
                "acme".to_string(),
                "pw".to_string(),
                587,
                "smtp.example.com".to_string()
            )]
        );
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 1, SmtpPhase::Applied)]]
        );
    }

    #[tokio::test]
    async fn the_password_put_is_kept_for_the_next_save_without_one() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );

        h.reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(
            h.secrets.applied().get("acme").map(String::as_str),
            Some("pw")
        );
    }

    #[tokio::test]
    async fn a_save_without_a_password_reuses_the_applied_one() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[], &[("acme", "old")]),
        );
        let reported = [status("acme", 1, SmtpPhase::Applied)];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 2)], &reported))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert_eq!(h.ferriskey.writes()[0].1, "old");
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 2, SmtpPhase::Applied)]]
        );
    }

    #[tokio::test]
    async fn a_save_with_nothing_to_send_as_password_fails_without_calling_ferriskey() {
        let h = harness(FakeFerriskey::default(), FakeSecrets::default());

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(
            h.repository.patches(),
            vec![vec![SmtpStatus {
                realm: "acme".to_string(),
                revision: 1,
                phase: SmtpPhase::Failed,
                message: Some("no password".to_string()),
            }]]
        );
    }

    #[tokio::test]
    async fn a_removal_deletes_the_config_and_forgets_the_password() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[], &[("acme", "old"), ("beta", "keep")]),
        );
        let reported = [status("acme", 1, SmtpPhase::Applied)];

        let outcome = h
            .reconciler
            .reconcile(&running(&[tombstone("acme", 2)], &reported))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert_eq!(h.ferriskey.calls(), vec!["delete acme"]);
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 2, SmtpPhase::Applied)]]
        );
        assert!(!h.secrets.applied().contains_key("acme"));
    }

    #[tokio::test]
    async fn a_removal_that_could_not_be_made_keeps_the_password_and_is_retried() {
        let h = harness(
            FakeFerriskey::failing(
                "delete acme",
                SmtpError::Retry {
                    message: "FerrisKey answered 503".to_string(),
                },
            ),
            FakeSecrets::with(&[], &[("acme", "old")]),
        );

        let outcome = h
            .reconciler
            .reconcile(&running(&[tombstone("acme", 2)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Retry);
        assert!(h.secrets.applied().contains_key("acme"));
    }

    #[tokio::test]
    async fn a_temporary_failure_is_pending_with_its_message_and_asks_for_a_retry() {
        let h = harness(
            FakeFerriskey::failing(
                "put acme",
                SmtpError::Retry {
                    message: "FerrisKey is unreachable".to_string(),
                },
            ),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Retry);
        assert_eq!(
            h.repository.patches(),
            vec![vec![SmtpStatus {
                realm: "acme".to_string(),
                revision: 1,
                phase: SmtpPhase::Pending,
                message: Some("FerrisKey is unreachable".to_string()),
            }]]
        );
        assert!(h.secrets.applied().is_empty());
    }

    #[tokio::test]
    async fn a_pending_entry_is_put_again_on_the_next_pass() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );
        let reported = [SmtpStatus {
            message: Some("FerrisKey is unreachable".to_string()),
            ..status("acme", 1, SmtpPhase::Pending)
        }];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &reported))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert_eq!(h.ferriskey.calls(), vec!["put acme"]);
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 1, SmtpPhase::Applied)]]
        );
    }

    #[tokio::test]
    async fn a_refusal_is_failed_with_ferriskeys_message_and_settles() {
        let h = harness(
            FakeFerriskey::failing(
                "put acme",
                SmtpError::Rejected {
                    message: "FerrisKey answered 400: bad host".to_string(),
                },
            ),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert_eq!(
            h.repository.patches(),
            vec![vec![SmtpStatus {
                realm: "acme".to_string(),
                revision: 1,
                phase: SmtpPhase::Failed,
                message: Some("FerrisKey answered 400: bad host".to_string()),
            }]]
        );
    }

    #[tokio::test]
    async fn a_failed_entry_is_not_tried_again_until_its_revision_moves() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );
        let reported = [SmtpStatus {
            message: Some("FerrisKey answered 400: bad host".to_string()),
            ..status("acme", 1, SmtpPhase::Failed)
        }];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &reported))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());

        h.reconciler
            .reconcile(&running(&[entry("acme", 2)], &reported))
            .await;

        assert_eq!(h.ferriskey.calls(), vec!["put acme"]);
    }

    #[tokio::test]
    async fn an_applied_entry_costs_no_call_and_no_write() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[("acme", "pw")]),
        );
        let reported = [status("acme", 3, SmtpPhase::Applied)];

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 3)], &reported))
            .await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn each_realm_is_gated_on_its_own_revision() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "a"), ("beta", "b")], &[]),
        );
        let reported = [
            status("acme", 3, SmtpPhase::Applied),
            status("beta", 1, SmtpPhase::Applied),
        ];

        h.reconciler
            .reconcile(&running(&[entry("acme", 3), entry("beta", 2)], &reported))
            .await;

        assert_eq!(h.ferriskey.calls(), vec!["put beta"]);
        assert_eq!(
            h.repository.patches(),
            vec![vec![
                status("acme", 3, SmtpPhase::Applied),
                status("beta", 2, SmtpPhase::Applied),
            ]]
        );
    }

    #[tokio::test]
    async fn status_entries_of_realms_that_left_the_spec_are_dropped() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[], &[("gone", "pw"), ("acme", "pw")]),
        );
        let reported = [
            status("acme", 1, SmtpPhase::Applied),
            status("gone", 4, SmtpPhase::Applied),
        ];

        h.reconciler
            .reconcile(&running(&[entry("acme", 1)], &reported))
            .await;

        assert!(h.ferriskey.calls().is_empty());
        assert_eq!(
            h.repository.patches(),
            vec![vec![status("acme", 1, SmtpPhase::Applied)]]
        );
        assert_eq!(h.secrets.applied().keys().collect::<Vec<_>>(), vec!["acme"]);
    }

    #[tokio::test]
    async fn an_emptied_spec_clears_what_was_reported() {
        let h = harness(FakeFerriskey::default(), FakeSecrets::default());
        let reported = [status("acme", 1, SmtpPhase::Applied)];

        h.reconciler.reconcile(&running(&[], &reported)).await;

        assert_eq!(h.repository.patches(), vec![Vec::<SmtpStatus>::new()]);
    }

    #[tokio::test]
    async fn an_instance_without_smtp_is_left_alone_entirely() {
        let secrets = FakeSecrets::default();
        secrets.state.lock().unwrap().unreadable = true;
        let h = harness(FakeFerriskey::default(), secrets);

        let outcome = h.reconciler.reconcile(&running(&[], &[])).await;

        assert_eq!(outcome, SmtpOutcome::Settled);
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn only_a_ready_running_ferriskey_instance_is_reconciled() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[]),
        );
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
                SmtpOutcome::Settled
            );
        }

        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn unreadable_secrets_ask_for_a_retry_and_touch_nothing() {
        let secrets = FakeSecrets::default();
        secrets.state.lock().unwrap().unreadable = true;
        let h = harness(FakeFerriskey::default(), secrets);

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Retry);
        assert!(h.ferriskey.calls().is_empty());
        assert!(h.repository.patches().is_empty());
    }

    #[tokio::test]
    async fn a_password_that_could_not_be_kept_leaves_the_entry_pending() {
        let secrets = FakeSecrets::with(&[("acme", "pw")], &[]);
        secrets.state.lock().unwrap().unwritable = true;
        let h = harness(FakeFerriskey::default(), secrets);

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Retry);
        assert_eq!(h.repository.patches()[0][0].phase, SmtpPhase::Pending);
    }

    #[tokio::test]
    async fn a_status_that_could_not_be_written_asks_for_a_retry() {
        let h = with_repository(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("acme", "pw")], &[]),
            FakeRepository {
                broken: true,
                ..Default::default()
            },
        );

        let outcome = h
            .reconciler
            .reconcile(&running(&[entry("acme", 1)], &[]))
            .await;

        assert_eq!(outcome, SmtpOutcome::Retry);
    }

    #[tokio::test]
    async fn the_status_has_one_entry_per_spec_entry_in_spec_order() {
        let h = harness(
            FakeFerriskey::default(),
            FakeSecrets::with(&[("b", "pw")], &[]),
        );

        h.reconciler
            .reconcile(&running(
                &[tombstone("z", 1), entry("b", 1), entry("a", 1)],
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
