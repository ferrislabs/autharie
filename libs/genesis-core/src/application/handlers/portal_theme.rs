use std::sync::Arc;

use tracing::{debug, info};

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::portal_theme_payload::PortalThemePayloadV1;
use crate::domain::error::GenesisError;
use crate::domain::ports::{BoxFuture, EventHandler, PortalThemePort};

pub struct PortalThemeEventHandler<P> {
    port: Arc<P>,
}

impl<P: PortalThemePort> PortalThemeEventHandler<P> {
    pub fn new(port: Arc<P>) -> Self {
        Self { port }
    }
}

impl<P: PortalThemePort> EventHandler for PortalThemeEventHandler<P> {
    fn routing_key(&self) -> &str {
        "deployment.portal_theme"
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = PortalThemePayloadV1::from_value(&event.payload)?;
            let reference =
                IdentityInstanceRef::for_deployment(payload.deployment_id, &payload.namespace);

            let applied = self.port.revision(&reference, &payload.realm).await?;
            if applied.is_some_and(|applied| payload.revision <= applied) {
                debug!(
                    deployment_id = %payload.deployment_id,
                    realm = %payload.realm,
                    revision = payload.revision,
                    applied,
                    "ignoring a portal theme revision that is not newer"
                );
                return Ok(());
            }

            info!(
                deployment_id = %payload.deployment_id,
                realm = %payload.realm,
                revision = payload.revision,
                "applying the portal theme"
            );

            self.port.apply(&reference, &payload).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    #[derive(Default)]
    struct FakeInstance {
        revisions: Mutex<Vec<(String, u64)>>,
        applied: Mutex<Vec<(String, u64)>>,
        references: Mutex<Vec<IdentityInstanceRef>>,
        fail_apply: Mutex<bool>,
    }

    impl PortalThemePort for FakeInstance {
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

        async fn apply<'a>(
            &'a self,
            reference: &'a IdentityInstanceRef,
            desired: &'a PortalThemePayloadV1,
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
            self.applied
                .lock()
                .expect("not poisoned")
                .push((desired.realm.clone(), desired.revision));
            Ok(())
        }
    }

    fn event(payload: Value) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::new_v4(),
            deployment_id: Some(DEPLOYMENT),
            dataplane_id: Uuid::new_v4(),
            routing_key: "deployment.portal_theme".to_string(),
            version: 1,
            payload,
            occurred_at: Utc::now(),
        }
    }

    fn theme(realm: &str, revision: u64) -> ActionEvent {
        event(json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": revision,
            "realm": realm,
            "config": { "colors": { "primary": "#112233" } },
        }))
    }

    fn applied(port: &FakeInstance) -> Vec<(String, u64)> {
        port.applied.lock().expect("not poisoned").clone()
    }

    #[tokio::test]
    async fn a_theme_reaches_the_instance_in_its_own_namespace() {
        let port = Arc::new(FakeInstance::default());
        let handler = PortalThemeEventHandler::new(port.clone());

        handler.handle(theme("master", 1)).await.expect("applied");

        assert_eq!(applied(&port), [("master".to_string(), 1)]);
        let references = port.references.lock().expect("not poisoned");
        assert_eq!(references[0].namespace, "tenant-a");
        assert_eq!(references[0].name, format!("deployment-{DEPLOYMENT}"));
    }

    #[tokio::test]
    async fn an_older_revision_changes_nothing() {
        let port = Arc::new(FakeInstance::default());
        let handler = PortalThemeEventHandler::new(port.clone());

        handler.handle(theme("master", 5)).await.expect("applied");
        handler
            .handle(theme("master", 4))
            .await
            .expect("ignored, not failed");

        assert_eq!(applied(&port), [("master".to_string(), 5)]);
    }

    #[tokio::test]
    async fn the_same_revision_redelivered_changes_nothing() {
        let port = Arc::new(FakeInstance::default());
        let handler = PortalThemeEventHandler::new(port.clone());

        handler.handle(theme("master", 5)).await.expect("applied");
        handler
            .handle(theme("master", 5))
            .await
            .expect("ignored, not failed");

        assert_eq!(applied(&port).len(), 1);
    }

    #[tokio::test]
    async fn a_newer_revision_is_applied() {
        let port = Arc::new(FakeInstance::default());
        let handler = PortalThemeEventHandler::new(port.clone());

        handler.handle(theme("master", 5)).await.expect("applied");
        handler.handle(theme("master", 6)).await.expect("applied");

        assert_eq!(applied(&port).len(), 2);
    }

    #[tokio::test]
    async fn another_realm_has_a_revision_of_its_own() {
        let port = Arc::new(FakeInstance::default());
        let handler = PortalThemeEventHandler::new(port.clone());

        handler.handle(theme("master", 9)).await.expect("applied");
        handler.handle(theme("acme", 1)).await.expect("applied");

        assert_eq!(
            applied(&port),
            [("master".to_string(), 9), ("acme".to_string(), 1)]
        );
    }

    #[tokio::test]
    async fn an_invalid_payload_never_reaches_the_port() {
        let port = Arc::new(FakeInstance::default());
        let handler = PortalThemeEventHandler::new(port.clone());
        let mut bad = theme("master", 1);
        bad.payload["config"] = json!([1]);

        let error = handler.handle(bad).await.expect_err("invalid");

        assert!(matches!(error, GenesisError::InvalidPayload { .. }));
        assert!(applied(&port).is_empty());
    }

    #[tokio::test]
    async fn a_failing_spec_write_is_an_error_so_the_action_is_retried() {
        let port = Arc::new(FakeInstance::default());
        *port.fail_apply.lock().expect("not poisoned") = true;
        let handler = PortalThemeEventHandler::new(port.clone());

        handler.handle(theme("master", 1)).await.expect_err("fails");

        assert!(port.revisions.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn the_handler_answers_to_its_routing_key() {
        let port = Arc::new(FakeInstance::default());

        assert_eq!(
            PortalThemeEventHandler::new(port).routing_key(),
            "deployment.portal_theme"
        );
    }
}
