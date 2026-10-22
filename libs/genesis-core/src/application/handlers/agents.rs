use std::sync::Arc;

use tracing::{debug, info};

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::agents_payload::AgentsPayloadV1;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::GenesisError;
use crate::domain::ports::{AgentsPort, BoxFuture, EventHandler};

pub struct AgentsEventHandler<P> {
    port: Arc<P>,
}

impl<P: AgentsPort> AgentsEventHandler<P> {
    pub fn new(port: Arc<P>) -> Self {
        Self { port }
    }
}

impl<P: AgentsPort> EventHandler for AgentsEventHandler<P> {
    fn routing_key(&self) -> &str {
        "deployment.agents"
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = AgentsPayloadV1::from_value(&event.payload)?;
            let reference =
                IdentityInstanceRef::for_deployment(payload.deployment_id, &payload.namespace);

            let applied = self.port.revision(&reference).await?;
            if payload.revision < applied {
                debug!(
                    deployment_id = %payload.deployment_id,
                    revision = payload.revision,
                    applied,
                    "ignoring an older agents revision"
                );
                return Ok(());
            }

            info!(
                deployment_id = %payload.deployment_id,
                revision = payload.revision,
                enabled = payload.enabled,
                policies = payload.policies.len(),
                "applying the agent settings"
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
    const POLICY: Uuid = Uuid::from_u128(2);
    const CEDAR: &str = "permit (principal, action, resource);";

    #[derive(Default)]
    struct FakeInstance {
        revision: Mutex<u64>,
        applied: Mutex<Vec<(IdentityInstanceRef, AgentsPayloadV1)>>,
        fail: Mutex<bool>,
    }

    impl AgentsPort for FakeInstance {
        async fn revision<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
        ) -> Result<u64, GenesisError> {
            Ok(*self.revision.lock().expect("not poisoned"))
        }

        async fn apply<'a>(
            &'a self,
            reference: &'a IdentityInstanceRef,
            desired: &'a AgentsPayloadV1,
        ) -> Result<(), GenesisError> {
            if *self.fail.lock().expect("not poisoned") {
                return Err(GenesisError::Kubernetes {
                    message: "down".to_string(),
                });
            }
            *self.revision.lock().expect("not poisoned") = desired.revision;
            self.applied
                .lock()
                .expect("not poisoned")
                .push((reference.clone(), desired.clone()));
            Ok(())
        }
    }

    fn event(payload: Value) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::new_v4(),
            deployment_id: Some(DEPLOYMENT),
            dataplane_id: Uuid::new_v4(),
            routing_key: "deployment.agents".to_string(),
            version: 1,
            payload,
            occurred_at: Utc::now(),
        }
    }

    fn agents(revision: u64, enabled: bool, name: &str) -> ActionEvent {
        event(json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "enabled": enabled,
            "realm": "master",
            "revision": revision,
            "policies": [{ "id": POLICY, "name": name, "cedar": CEDAR }],
        }))
    }

    #[tokio::test]
    async fn the_desired_settings_reach_the_instance_in_its_own_namespace() {
        let port = Arc::new(FakeInstance::default());
        let handler = AgentsEventHandler::new(port.clone());

        handler.handle(agents(1, true, "a")).await.expect("applied");

        let applied = port.applied.lock().expect("not poisoned");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].0.namespace, "tenant-a");
        assert_eq!(applied[0].0.name, format!("deployment-{DEPLOYMENT}"));
        assert_eq!(applied[0].1.revision, 1);
        assert!(applied[0].1.enabled);
        assert_eq!(applied[0].1.policies[0].cedar, CEDAR);
    }

    #[tokio::test]
    async fn an_older_revision_changes_nothing() {
        let port = Arc::new(FakeInstance::default());
        let handler = AgentsEventHandler::new(port.clone());

        handler
            .handle(agents(5, true, "new"))
            .await
            .expect("applied");
        handler
            .handle(agents(4, false, "old"))
            .await
            .expect("ignored, not failed");

        let applied = port.applied.lock().expect("not poisoned");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].1.policies[0].name, "new");
        assert_eq!(*port.revision.lock().expect("not poisoned"), 5);
    }

    #[tokio::test]
    async fn the_same_revision_redelivered_is_applied_again() {
        let port = Arc::new(FakeInstance::default());
        let handler = AgentsEventHandler::new(port.clone());

        handler.handle(agents(5, true, "a")).await.expect("applied");
        handler.handle(agents(5, true, "a")).await.expect("applied");

        assert_eq!(port.applied.lock().expect("not poisoned").len(), 2);
    }

    #[tokio::test]
    async fn turning_agents_off_is_applied_like_any_revision() {
        let port = Arc::new(FakeInstance::default());
        let handler = AgentsEventHandler::new(port.clone());

        handler.handle(agents(1, true, "a")).await.expect("applied");
        handler
            .handle(agents(2, false, "a"))
            .await
            .expect("applied");

        let applied = port.applied.lock().expect("not poisoned");
        assert!(!applied[1].1.enabled);
        assert_eq!(*port.revision.lock().expect("not poisoned"), 2);
    }

    #[tokio::test]
    async fn an_invalid_payload_never_reaches_the_port() {
        let port = Arc::new(FakeInstance::default());
        let handler = AgentsEventHandler::new(port.clone());
        let mut bad = agents(1, true, "a");
        bad.payload["realm"] = json!("a/b");

        let error = handler.handle(bad).await.expect_err("invalid");

        assert!(matches!(error, GenesisError::InvalidPayload { .. }));
        assert!(port.applied.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_failing_write_is_an_error_so_the_action_is_retried() {
        let port = Arc::new(FakeInstance::default());
        *port.fail.lock().expect("not poisoned") = true;
        let handler = AgentsEventHandler::new(port.clone());

        let error = handler
            .handle(agents(1, true, "a"))
            .await
            .expect_err("fails");

        assert!(matches!(error, GenesisError::Kubernetes { .. }));
    }

    #[test]
    fn the_handler_answers_to_the_agents_routing_key() {
        let port = Arc::new(FakeInstance::default());

        assert_eq!(
            AgentsEventHandler::new(port).routing_key(),
            "deployment.agents"
        );
    }
}
