use std::sync::Arc;

use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::iam_payload::IamRequestPayloadV1;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::GenesisError;
use crate::domain::ports::{BoxFuture, EventHandler, IamPort};

pub struct IamRequestEventHandler<P> {
    port: Arc<P>,
    now: fn() -> DateTime<Utc>,
}

impl<P: IamPort> IamRequestEventHandler<P> {
    pub fn new(port: Arc<P>) -> Self {
        Self::with_clock(port, Utc::now)
    }

    pub fn with_clock(port: Arc<P>, now: fn() -> DateTime<Utc>) -> Self {
        Self { port, now }
    }
}

impl<P: IamPort> EventHandler for IamRequestEventHandler<P> {
    fn routing_key(&self) -> &str {
        "deployment.iam.request"
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = IamRequestPayloadV1::from_value(&event.payload)?;

            if payload.entry.expires_at <= (self.now)() {
                debug!(
                    deployment_id = %payload.deployment_id,
                    request_id = %payload.entry.request_id,
                    "dropping an IAM request that expired before it was written"
                );
                return Ok(());
            }

            let reference =
                IdentityInstanceRef::for_deployment(payload.deployment_id, &payload.namespace);

            info!(
                deployment_id = %payload.deployment_id,
                request_id = %payload.entry.request_id,
                "asking the operator for an IAM request"
            );

            self.port.request(&reference, &payload.entry).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::iam_payload::IamRequestEntry;
    use chrono::TimeZone;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const REQUEST: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const SEARCH: &str = "jane.doe@example.com";

    #[derive(Default)]
    struct FakeInstance {
        requests: Mutex<Vec<(IdentityInstanceRef, IamRequestEntry)>>,
        fail: Mutex<bool>,
    }

    impl IamPort for FakeInstance {
        async fn request<'a>(
            &'a self,
            reference: &'a IdentityInstanceRef,
            request: &'a IamRequestEntry,
        ) -> Result<(), GenesisError> {
            if *self.fail.lock().expect("not poisoned") {
                return Err(GenesisError::Kubernetes {
                    message: "down".to_string(),
                });
            }
            self.requests
                .lock()
                .expect("not poisoned")
                .push((reference.clone(), request.clone()));
            Ok(())
        }

        async fn drop_requests<'a>(
            &'a self,
            _reference: &'a IdentityInstanceRef,
            _request_ids: &'a [&'a str],
        ) -> Result<(), GenesisError> {
            Ok(())
        }
    }

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap()
    }

    fn handler(port: &Arc<FakeInstance>) -> IamRequestEventHandler<FakeInstance> {
        IamRequestEventHandler::with_clock(Arc::clone(port), noon)
    }

    fn event(payload: Value) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::new_v4(),
            deployment_id: Some(DEPLOYMENT),
            dataplane_id: Uuid::new_v4(),
            routing_key: "deployment.iam.request".to_string(),
            version: 1,
            payload,
            occurred_at: Utc::now(),
        }
    }

    fn payload(expires_at: &str) -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "request_id": REQUEST,
            "realm": "master",
            "operation": { "type": "applications.list", "page": 1, "limit": 25, "search": SEARCH },
            "sealing_key": { "kty": "RSA", "n": "AQAB", "e": "AQAB", "alg": "RSA-OAEP-256" },
            "expires_at": expires_at,
        })
    }

    #[test]
    fn the_handler_answers_to_the_frozen_routing_key() {
        let port = Arc::new(FakeInstance::default());

        assert_eq!(handler(&port).routing_key(), "deployment.iam.request");
    }

    #[tokio::test]
    async fn spec_iam_10_the_request_is_written_to_the_deployments_own_instance() {
        let port = Arc::new(FakeInstance::default());

        handler(&port)
            .handle(event(payload("2026-10-24T12:05:00Z")))
            .await
            .expect("written");

        let requests = port.requests.lock().expect("not poisoned");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0.namespace, "tenant-a");
        assert_eq!(requests[0].0.name, format!("deployment-{DEPLOYMENT}"));
        assert_eq!(requests[0].1.request_id, REQUEST);
        assert_eq!(requests[0].1.realm, "master");
    }

    #[tokio::test]
    async fn a_request_that_has_already_expired_is_not_written() {
        let port = Arc::new(FakeInstance::default());

        handler(&port)
            .handle(event(payload("2026-10-24T11:59:59Z")))
            .await
            .expect("dropped, not failed");
        handler(&port)
            .handle(event(payload("2026-10-24T12:00:00Z")))
            .await
            .expect("dropped, not failed");

        assert!(port.requests.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn an_invalid_payload_never_reaches_the_port() {
        let port = Arc::new(FakeInstance::default());
        let mut bad = payload("2026-10-24T12:05:00Z");
        bad["operation"] = json!({ "type": "users.delete" });

        let error = handler(&port)
            .handle(event(bad))
            .await
            .expect_err("invalid");

        assert!(matches!(error, GenesisError::InvalidPayload { .. }));
        assert!(port.requests.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_failing_write_is_an_error_so_the_action_is_retried_without_leaking_the_search() {
        let port = Arc::new(FakeInstance::default());
        *port.fail.lock().expect("not poisoned") = true;

        let error = handler(&port)
            .handle(event(payload("2026-10-24T12:05:00Z")))
            .await
            .expect_err("fails");

        assert!(!error.to_string().contains(SEARCH));
    }
}
