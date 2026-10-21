use autharie_core::{CoreError, billing::BillingError};
use axum::{body::Bytes, extract::State, http::HeaderMap};
use axum_extra::routing::TypedPath;
use chrono::Utc;
use serde::Serialize;
use tracing::warn;
use utoipa::ToSchema;

use crate::{errors::ApiError, response::Response, state::AppState};

const SIGNATURE: &str = "Stripe-Signature";

#[derive(Serialize, ToSchema, PartialEq, Debug)]
pub struct ProviderEventsResponse {
    received: bool,
}

#[derive(TypedPath)]
#[typed_path("/billing/provider-events")]
pub struct ProviderEventsRoute;

#[utoipa::path(
    post,
    path = "/provider-events",
    summary = "receive a payment provider event",
    tag = "billing",
    description = "Called by the payment provider, never by a user: the request carries no bearer token and is trusted only through the signature of its body. An event is applied once, however many times the provider delivers it.",
    params(
        ("Stripe-Signature" = String, Header, description = "The provider's signature of the raw body")
    ),
    request_body(
        content = String,
        content_type = "application/json",
        description = "The event, exactly as the provider sent it"
    ),
    responses(
        (status = 200, description = "The event was applied, or had been already", body = ProviderEventsResponse),
        (status = 400, description = "The signature is missing or does not match the body", body = ApiError),
        (status = 502, description = "The payment provider failed while the event was applied; it will deliver the event again", body = ApiError),
        (status = 503, description = "Billing is not enabled on this installation", body = ApiError)
    )
)]
pub async fn provider_events_handler(
    _: ProviderEventsRoute,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response<ProviderEventsResponse>, ApiError> {
    let runtime = state
        .billing
        .as_deref()
        .ok_or_else(|| ApiError::ServiceUnavailable {
            reason: "billing is not enabled on this installation".to_string(),
        })?;

    let signature = headers
        .get(SIGNATURE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::BadRequest {
            reason: format!("the {SIGNATURE} header is missing"),
        })?;

    state
        .service
        .apply_provider_event(&runtime.provider, &body, signature, Utc::now())
        .await
        .inspect_err(|error| {
            if matches!(
                error,
                CoreError::Billing(BillingError::InvalidEventSignature)
            ) {
                warn!("a payment provider event was refused: its signature does not match");
            }
        })?;

    Ok(Response::OK(ProviderEventsResponse { received: true }))
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use axum::http::HeaderValue;

    use super::*;
    use crate::{args::BillingArgs, billing::BillingRuntime, test_helpers::app_state};

    fn enabled_state() -> AppState {
        let runtime = BillingRuntime::from_args(&BillingArgs {
            enabled: true,
            first_period: Some("2026-12".to_string()),
            return_url: Some("https://console.test/billing".to_string()),
            stripe_secret_key: Some("sk_test_x".to_string()),
            stripe_webhook_secret: Some("whsec_x".to_string()),
            ..BillingArgs::default()
        })
        .expect("a runtime")
        .expect("billing is enabled");

        AppState {
            billing: Some(Arc::new(runtime)),
            ..app_state()
        }
    }

    fn signed_with(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(SIGNATURE, HeaderValue::from_static(value));
        headers
    }

    #[tokio::test]
    async fn without_billing_the_endpoint_is_unavailable() {
        let result = provider_events_handler(
            ProviderEventsRoute,
            State(app_state()),
            signed_with("t=1,v1=00"),
            Bytes::from_static(b"{}"),
        )
        .await;

        assert!(matches!(
            result,
            Err(ApiError::ServiceUnavailable { reason }) if reason == "billing is not enabled on this installation"
        ));
    }

    #[tokio::test]
    async fn a_request_without_a_signature_is_a_bad_request() {
        let result = provider_events_handler(
            ProviderEventsRoute,
            State(enabled_state()),
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest { .. })));
    }

    #[tokio::test]
    async fn a_signature_that_does_not_match_is_a_bad_request_and_touches_no_database() {
        let result = provider_events_handler(
            ProviderEventsRoute,
            State(enabled_state()),
            signed_with("t=1,v1=00"),
            Bytes::from_static(b"{}"),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest { .. })));
    }

    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("not poisoned")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn captured(buffer: &Arc<Mutex<Vec<u8>>>) -> tracing::subscriber::DefaultGuard {
        let sink = Arc::clone(buffer);
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(move || Sink(Arc::clone(&sink)))
                .finish(),
        )
    }

    #[tokio::test]
    async fn a_refused_signature_is_warned_about_without_the_payload_nor_the_header() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _guard = captured(&buffer);

        let result = provider_events_handler(
            ProviderEventsRoute,
            State(enabled_state()),
            signed_with("t=1,v1=header-marker"),
            Bytes::from_static(br#"{"payload":"payload-marker"}"#),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest { .. })));
        let logged =
            String::from_utf8(buffer.lock().expect("not poisoned").clone()).expect("utf-8 logs");
        assert!(logged.contains("WARN"), "{logged}");
        assert!(logged.contains("signature"), "{logged}");
        assert!(!logged.contains("payload-marker"), "{logged}");
        assert!(!logged.contains("header-marker"), "{logged}");
    }

    #[tokio::test]
    async fn a_request_without_a_signature_header_logs_no_refused_signature() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _guard = captured(&buffer);

        let result = provider_events_handler(
            ProviderEventsRoute,
            State(enabled_state()),
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest { .. })));
        assert!(buffer.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn the_answer_is_received_true() {
        let body =
            serde_json::to_value(ProviderEventsResponse { received: true }).expect("a response");

        assert_eq!(body, serde_json::json!({"received": true}));
    }
}
