use std::sync::Arc;

use tracing::{debug, info};

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::webhooks_payload::{
    WebhookRequestKind, WebhookRequestPayload, WebhooksPayloadV1,
};
use crate::domain::error::GenesisError;
use crate::domain::ports::{BoxFuture, EventHandler, WebhookPort};

pub struct WebhooksEventHandler<P> {
    port: Arc<P>,
}

impl<P: WebhookPort> WebhooksEventHandler<P> {
    pub fn new(port: Arc<P>) -> Self {
        Self { port }
    }
}

impl<P: WebhookPort> EventHandler for WebhooksEventHandler<P> {
    fn routing_key(&self) -> &str {
        "deployment.webhooks"
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = WebhooksPayloadV1::from_value(&event.payload)?;
            let reference =
                IdentityInstanceRef::for_deployment(payload.deployment_id, &payload.namespace);

            let applied = self.port.revision(&reference).await?;
            if payload.revision < applied {
                debug!(
                    deployment_id = %payload.deployment_id,
                    revision = payload.revision,
                    applied,
                    "ignoring an older webhooks revision"
                );
                return Ok(());
            }

            info!(
                deployment_id = %payload.deployment_id,
                revision = payload.revision,
                webhooks = payload.webhooks.len(),
                "applying the webhooks"
            );

            self.port.apply(&reference, &payload).await
        })
    }
}

pub struct WebhookRequestEventHandler<P> {
    port: Arc<P>,
    kind: WebhookRequestKind,
}

impl<P: WebhookPort> WebhookRequestEventHandler<P> {
    pub fn rotate_secret(port: Arc<P>) -> Self {
        Self {
            port,
            kind: WebhookRequestKind::RotateSecret,
        }
    }

    pub fn retry_delivery(port: Arc<P>) -> Self {
        Self {
            port,
            kind: WebhookRequestKind::RetryDelivery,
        }
    }
}

impl<P: WebhookPort> EventHandler for WebhookRequestEventHandler<P> {
    fn routing_key(&self) -> &str {
        match self.kind {
            WebhookRequestKind::RotateSecret => "deployment.webhook.rotate_secret",
            WebhookRequestKind::RetryDelivery => "deployment.webhook.retry_delivery",
        }
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = match self.kind {
                WebhookRequestKind::RotateSecret => {
                    WebhookRequestPayload::rotate_from_value(&event.payload)?
                }
                WebhookRequestKind::RetryDelivery => {
                    WebhookRequestPayload::retry_from_value(&event.payload)?
                }
            };
            let reference =
                IdentityInstanceRef::for_deployment(payload.deployment_id, &payload.namespace);

            info!(
                deployment_id = %payload.deployment_id,
                request_id = %payload.entry.id,
                kind = ?payload.entry.kind,
                "asking the operator for a webhook request"
            );

            self.port.request(&reference, &payload.entry).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::webhooks_payload::WebhookRequestEntry;
    use chrono::Utc;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const HOOK: Uuid = Uuid::from_u128(2);
    const REQUEST: Uuid = Uuid::from_u128(3);
    const VALUE: &str = "Bearer s3cr3t-value";

    #[derive(Default)]
    struct FakeInstance {
        revision: Mutex<u64>,
        applied: Mutex<Vec<(IdentityInstanceRef, WebhooksPayloadV1)>>,
        requests: Mutex<Vec<WebhookRequestEntry>>,
        fail: Mutex<bool>,
    }

    impl WebhookPort for FakeInstance {
        async fn revision<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
        ) -> Result<u64, GenesisError> {
            Ok(*self.revision.lock().expect("not poisoned"))
        }

        async fn apply<'a>(
            &'a self,
            reference: &'a IdentityInstanceRef,
            desired: &'a WebhooksPayloadV1,
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

        async fn request<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            request: &'a WebhookRequestEntry,
        ) -> Result<(), GenesisError> {
            self.requests
                .lock()
                .expect("not poisoned")
                .push(request.clone());
            Ok(())
        }
    }

    fn event(routing_key: &str, payload: Value) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::new_v4(),
            deployment_id: Some(DEPLOYMENT),
            dataplane_id: Uuid::new_v4(),
            routing_key: routing_key.to_string(),
            version: 1,
            payload,
            occurred_at: Utc::now(),
        }
    }

    fn webhooks(revision: u64, name: &str) -> ActionEvent {
        event(
            "deployment.webhooks",
            json!({
                "deployment_id": DEPLOYMENT,
                "namespace": "tenant-a",
                "revision": revision,
                "webhooks": [{
                    "id": HOOK,
                    "realm": "master",
                    "name": name,
                    "endpoint": "https://hooks.example.com",
                    "events": ["user.created"],
                    "header_names": ["Authorization"],
                }],
                "headers": { HOOK.to_string(): { "Authorization": VALUE } },
            }),
        )
    }

    #[tokio::test]
    async fn the_desired_set_reaches_the_instance_in_its_own_namespace() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhooksEventHandler::new(port.clone());

        handler.handle(webhooks(1, "a")).await.expect("applied");

        let applied = port.applied.lock().expect("not poisoned");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].0.namespace, "tenant-a");
        assert_eq!(applied[0].0.name, format!("deployment-{DEPLOYMENT}"));
        assert_eq!(applied[0].1.revision, 1);
    }

    #[tokio::test]
    async fn an_older_revision_changes_nothing() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhooksEventHandler::new(port.clone());

        handler.handle(webhooks(5, "new")).await.expect("applied");
        handler
            .handle(webhooks(4, "old"))
            .await
            .expect("ignored, not failed");

        let applied = port.applied.lock().expect("not poisoned");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].1.webhooks[0].name, "new");
        assert_eq!(*port.revision.lock().expect("not poisoned"), 5);
    }

    #[tokio::test]
    async fn the_same_revision_redelivered_is_applied_again() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhooksEventHandler::new(port.clone());

        handler.handle(webhooks(5, "a")).await.expect("applied");
        handler.handle(webhooks(5, "a")).await.expect("applied");

        assert_eq!(port.applied.lock().expect("not poisoned").len(), 2);
    }

    #[tokio::test]
    async fn a_newer_revision_replaces_the_older_one() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhooksEventHandler::new(port.clone());

        handler.handle(webhooks(1, "a")).await.expect("applied");
        handler.handle(webhooks(2, "b")).await.expect("applied");

        assert_eq!(*port.revision.lock().expect("not poisoned"), 2);
    }

    #[tokio::test]
    async fn an_invalid_payload_never_reaches_the_port() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhooksEventHandler::new(port.clone());
        let mut bad = webhooks(1, "a");
        bad.payload["webhooks"][0]["endpoint"] = json!("http://insecure");

        let error = handler.handle(bad).await.expect_err("invalid");

        assert!(matches!(error, GenesisError::InvalidPayload { .. }));
        assert!(port.applied.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_failing_write_is_an_error_so_the_action_is_retried_without_leaking_values() {
        let port = Arc::new(FakeInstance::default());
        *port.fail.lock().expect("not poisoned") = true;
        let handler = WebhooksEventHandler::new(port.clone());

        let error = handler.handle(webhooks(1, "a")).await.expect_err("fails");

        assert!(!error.to_string().contains(VALUE));
    }

    fn request_payload() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "webhook_id": HOOK,
            "request_id": REQUEST,
        })
    }

    #[tokio::test]
    async fn a_rotation_is_recorded_as_a_request_with_its_id() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhookRequestEventHandler::rotate_secret(port.clone());

        handler
            .handle(event("deployment.webhook.rotate_secret", request_payload()))
            .await
            .expect("recorded");

        let requests = port.requests.lock().expect("not poisoned");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, REQUEST.to_string());
        assert_eq!(requests[0].kind, WebhookRequestKind::RotateSecret);
        assert_eq!(requests[0].webhook_id, HOOK.to_string());
    }

    #[tokio::test]
    async fn a_retry_is_recorded_with_its_delivery() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhookRequestEventHandler::retry_delivery(port.clone());
        let mut payload = request_payload();
        payload["delivery_id"] = json!("d-9");

        handler
            .handle(event("deployment.webhook.retry_delivery", payload))
            .await
            .expect("recorded");

        let requests = port.requests.lock().expect("not poisoned");
        assert_eq!(requests[0].kind, WebhookRequestKind::RetryDelivery);
        assert_eq!(requests[0].delivery_id.as_deref(), Some("d-9"));
    }

    #[tokio::test]
    async fn a_retry_without_a_delivery_is_refused() {
        let port = Arc::new(FakeInstance::default());
        let handler = WebhookRequestEventHandler::retry_delivery(port.clone());

        assert!(
            handler
                .handle(event(
                    "deployment.webhook.retry_delivery",
                    request_payload()
                ))
                .await
                .is_err()
        );
        assert!(port.requests.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn each_handler_answers_to_its_routing_key() {
        let port = Arc::new(FakeInstance::default());

        assert_eq!(
            WebhooksEventHandler::new(port.clone()).routing_key(),
            "deployment.webhooks"
        );
        assert_eq!(
            WebhookRequestEventHandler::rotate_secret(port.clone()).routing_key(),
            "deployment.webhook.rotate_secret"
        );
        assert_eq!(
            WebhookRequestEventHandler::retry_delivery(port).routing_key(),
            "deployment.webhook.retry_delivery"
        );
    }
}
