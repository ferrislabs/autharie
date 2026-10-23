use std::sync::Arc;

use tracing::{debug, info};

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::smtp_payload::SmtpPayloadV1;
use crate::domain::error::GenesisError;
use crate::domain::ports::{BoxFuture, EventHandler, SmtpPort};

pub struct SmtpEventHandler<P> {
    port: Arc<P>,
}

impl<P: SmtpPort> SmtpEventHandler<P> {
    pub fn new(port: Arc<P>) -> Self {
        Self { port }
    }
}

impl<P: SmtpPort> EventHandler for SmtpEventHandler<P> {
    fn routing_key(&self) -> &str {
        "deployment.smtp"
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = SmtpPayloadV1::from_value(&event.payload)?;
            let reference =
                IdentityInstanceRef::for_deployment(payload.deployment_id, &payload.namespace);

            let applied = self.port.revision(&reference, &payload.realm).await?;
            if applied.is_some_and(|applied| payload.revision <= applied) {
                debug!(
                    deployment_id = %payload.deployment_id,
                    realm = %payload.realm,
                    revision = payload.revision,
                    applied,
                    "ignoring an smtp revision that is not newer"
                );
                return Ok(());
            }

            info!(
                deployment_id = %payload.deployment_id,
                realm = %payload.realm,
                revision = payload.revision,
                removal = payload.smtp.is_none(),
                "applying the smtp settings"
            );

            if let Some(change) = payload.password_change() {
                self.port
                    .write_password(&reference, &payload.realm, change)
                    .await?;
            }

            self.port.apply(&reference, &payload).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::smtp_payload::PasswordChange;
    use chrono::Utc;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const PASSWORD: &str = "hunter2-do-not-print";

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Password(String, Option<String>),
        Apply(u64),
    }

    #[derive(Default)]
    struct FakeInstance {
        revisions: Mutex<Vec<(String, u64)>>,
        calls: Mutex<Vec<Call>>,
        references: Mutex<Vec<IdentityInstanceRef>>,
        fail_password: Mutex<bool>,
        fail_apply: Mutex<bool>,
    }

    impl SmtpPort for FakeInstance {
        async fn revision<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            realm: &'a str,
        ) -> Result<Option<u64>, GenesisError> {
            Ok(self
                .revisions
                .lock()
                .expect("not poisoned")
                .iter()
                .find(|(known, _)| known == realm)
                .map(|(_, revision)| *revision))
        }

        async fn write_password<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            realm: &'a str,
            change: PasswordChange<'a>,
        ) -> Result<(), GenesisError> {
            if *self.fail_password.lock().expect("not poisoned") {
                return Err(GenesisError::Kubernetes {
                    message: "down".to_string(),
                });
            }
            let value = match change {
                PasswordChange::Set(password) => Some(password.to_string()),
                PasswordChange::Remove => None,
            };
            self.calls
                .lock()
                .expect("not poisoned")
                .push(Call::Password(realm.to_string(), value));
            Ok(())
        }

        async fn apply<'a>(
            &'a self,
            reference: &'a IdentityInstanceRef,
            desired: &'a SmtpPayloadV1,
        ) -> Result<(), GenesisError> {
            if *self.fail_apply.lock().expect("not poisoned") {
                return Err(GenesisError::Kubernetes {
                    message: "down".to_string(),
                });
            }
            let mut revisions = self.revisions.lock().expect("not poisoned");
            revisions.retain(|(known, _)| known != &desired.realm);
            revisions.push((desired.realm.clone(), desired.revision));
            self.references
                .lock()
                .expect("not poisoned")
                .push(reference.clone());
            self.calls
                .lock()
                .expect("not poisoned")
                .push(Call::Apply(desired.revision));
            Ok(())
        }
    }

    fn event(payload: Value) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::new_v4(),
            deployment_id: Some(DEPLOYMENT),
            dataplane_id: Uuid::new_v4(),
            routing_key: "deployment.smtp".to_string(),
            version: 1,
            payload,
            occurred_at: Utc::now(),
        }
    }

    fn save(revision: u64, password: Value) -> ActionEvent {
        event(json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": revision,
            "realm": "master",
            "smtp": {
                "host": "smtp.example.com",
                "port": 587,
                "encryption": "tls",
                "username": "mailer",
                "from_email": "no-reply@example.com",
                "from_name": "Example",
                "password": password,
            },
        }))
    }

    fn removal(revision: u64) -> ActionEvent {
        event(json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": revision,
            "realm": "master",
            "smtp": null,
        }))
    }

    fn password_call(password: Option<&str>) -> Call {
        Call::Password("master".to_string(), password.map(str::to_string))
    }

    #[tokio::test]
    async fn the_password_is_written_before_the_spec_so_the_operator_never_sees_a_revision_without_it()
     {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler
            .handle(save(1, json!(PASSWORD)))
            .await
            .expect("applied");

        assert_eq!(
            *port.calls.lock().expect("not poisoned"),
            [password_call(Some(PASSWORD)), Call::Apply(1)]
        );
    }

    #[tokio::test]
    async fn the_settings_reach_the_instance_in_its_own_namespace() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler
            .handle(save(1, json!(PASSWORD)))
            .await
            .expect("applied");

        let references = port.references.lock().expect("not poisoned");
        assert_eq!(references[0].namespace, "tenant-a");
        assert_eq!(references[0].name, format!("deployment-{DEPLOYMENT}"));
    }

    #[tokio::test]
    async fn a_save_without_a_password_leaves_the_secret_alone() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler.handle(save(1, Value::Null)).await.expect("applied");

        assert_eq!(*port.calls.lock().expect("not poisoned"), [Call::Apply(1)]);
    }

    #[tokio::test]
    async fn a_removal_deletes_the_key_before_the_tombstone_is_written() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler.handle(removal(2)).await.expect("applied");

        assert_eq!(
            *port.calls.lock().expect("not poisoned"),
            [password_call(None), Call::Apply(2)]
        );
    }

    #[tokio::test]
    async fn an_older_revision_changes_nothing_not_even_the_password() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler
            .handle(save(5, json!("new")))
            .await
            .expect("applied");
        handler
            .handle(save(4, json!("old")))
            .await
            .expect("ignored, not failed");

        assert_eq!(
            *port.calls.lock().expect("not poisoned"),
            [password_call(Some("new")), Call::Apply(5)]
        );
    }

    #[tokio::test]
    async fn the_same_revision_redelivered_changes_nothing() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler
            .handle(save(5, json!(PASSWORD)))
            .await
            .expect("applied");
        handler
            .handle(save(5, json!(PASSWORD)))
            .await
            .expect("ignored, not failed");

        assert_eq!(port.calls.lock().expect("not poisoned").len(), 2);
    }

    #[tokio::test]
    async fn an_older_save_cannot_bring_back_a_removed_provider_or_its_password() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler.handle(removal(6)).await.expect("applied");
        handler
            .handle(save(5, json!(PASSWORD)))
            .await
            .expect("ignored, not failed");

        assert_eq!(
            *port.calls.lock().expect("not poisoned"),
            [password_call(None), Call::Apply(6)]
        );
    }

    #[tokio::test]
    async fn a_newer_save_after_a_removal_is_applied() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());

        handler.handle(removal(6)).await.expect("applied");
        handler
            .handle(save(7, json!(PASSWORD)))
            .await
            .expect("applied");

        assert_eq!(port.calls.lock().expect("not poisoned").len(), 4);
    }

    #[tokio::test]
    async fn another_realm_has_a_revision_of_its_own() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());
        let mut other = save(1, json!(PASSWORD));
        other.payload["realm"] = json!("acme");

        handler
            .handle(save(9, json!(PASSWORD)))
            .await
            .expect("applied");
        handler.handle(other).await.expect("applied");

        assert_eq!(port.calls.lock().expect("not poisoned").len(), 4);
    }

    #[tokio::test]
    async fn an_invalid_payload_never_reaches_the_port() {
        let port = Arc::new(FakeInstance::default());
        let handler = SmtpEventHandler::new(port.clone());
        let mut bad = save(1, json!(PASSWORD));
        bad.payload["smtp"]["port"] = json!(0);

        let error = handler.handle(bad).await.expect_err("invalid");

        assert!(matches!(error, GenesisError::InvalidPayload { .. }));
        assert!(port.calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_failing_password_write_stops_before_the_spec_and_leaks_nothing() {
        let port = Arc::new(FakeInstance::default());
        *port.fail_password.lock().expect("not poisoned") = true;
        let handler = SmtpEventHandler::new(port.clone());

        let error = handler
            .handle(save(1, json!(PASSWORD)))
            .await
            .expect_err("fails");

        assert!(!error.to_string().contains(PASSWORD));
        assert!(port.calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_failing_spec_write_is_an_error_so_the_action_is_retried() {
        let port = Arc::new(FakeInstance::default());
        *port.fail_apply.lock().expect("not poisoned") = true;
        let handler = SmtpEventHandler::new(port.clone());

        let error = handler
            .handle(save(1, json!(PASSWORD)))
            .await
            .expect_err("fails");

        assert!(!error.to_string().contains(PASSWORD));
        assert!(port.revisions.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn the_handler_answers_to_its_routing_key() {
        let port = Arc::new(FakeInstance::default());

        assert_eq!(SmtpEventHandler::new(port).routing_key(), "deployment.smtp");
    }
}
