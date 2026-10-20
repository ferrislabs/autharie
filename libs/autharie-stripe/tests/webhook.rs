mod common;

use autharie_domain::billing::ports::PaymentProvider;
use autharie_domain::billing::{
    BillingError, ProviderCustomerId, ProviderEventId, ProviderEventKind, ProviderInvoiceId,
    ProviderPaymentMethodId,
};
use autharie_stripe::StripePaymentProvider;
use chrono::{DateTime, TimeZone, Utc};
use common::{WEBHOOK_SECRET, config};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

fn provider() -> StripePaymentProvider {
    StripePaymentProvider::new(config("http://127.0.0.1:1".to_string())).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 12, 1, 12, 0, 0).unwrap()
}

fn mac_with(secret: &str, timestamp: i64, payload: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

fn header_at(timestamp: i64, payload: &[u8]) -> String {
    format!(
        "t={timestamp},v1={}",
        mac_with(WEBHOOK_SECRET, timestamp, payload)
    )
}

fn event(kind: &str, object: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "evt_42",
        "object": "event",
        "type": kind,
        "data": {"object": object}
    }))
    .unwrap()
}

fn read(payload: &[u8]) -> Result<ProviderEventKind, BillingError> {
    let header = header_at(now().timestamp(), payload);
    provider()
        .read_event(payload, &header, now())
        .map(|event| event.kind)
}

#[test]
fn a_genuine_signature_is_accepted() {
    let payload = event("customer.updated", json!({"id": "cus_1"}));

    let read = provider()
        .read_event(&payload, &header_at(now().timestamp(), &payload), now())
        .unwrap();

    assert_eq!(read.id, ProviderEventId::parse("evt_42").unwrap());
}

#[test]
fn a_signature_made_with_another_secret_is_refused() {
    let payload = event("customer.updated", json!({}));
    let timestamp = now().timestamp();
    let header = format!(
        "t={timestamp},v1={}",
        mac_with("whsec_other", timestamp, &payload)
    );

    assert_eq!(
        provider().read_event(&payload, &header, now()),
        Err(BillingError::InvalidEventSignature)
    );
}

#[test]
fn a_payload_changed_after_signing_is_refused() {
    let payload = event("invoice.paid", json!({"id": "in_1"}));
    let header = header_at(now().timestamp(), &payload);
    let tampered = event("invoice.paid", json!({"id": "in_2"}));

    assert_eq!(
        provider().read_event(&tampered, &header, now()),
        Err(BillingError::InvalidEventSignature)
    );
}

#[test]
fn a_timestamp_changed_after_signing_is_refused() {
    let payload = event("customer.updated", json!({}));
    let timestamp = now().timestamp();
    let signature = mac_with(WEBHOOK_SECRET, timestamp, &payload);
    let header = format!("t={},v1={signature}", timestamp + 1);

    assert_eq!(
        provider().read_event(&payload, &header, now()),
        Err(BillingError::InvalidEventSignature)
    );
}

#[test]
fn a_timestamp_301_seconds_old_is_refused() {
    let payload = event("customer.updated", json!({}));
    let header = header_at(now().timestamp() - 301, &payload);

    assert_eq!(
        provider().read_event(&payload, &header, now()),
        Err(BillingError::InvalidEventSignature)
    );
}

#[test]
fn a_timestamp_301_seconds_ahead_is_refused() {
    let payload = event("customer.updated", json!({}));
    let header = header_at(now().timestamp() + 301, &payload);

    assert_eq!(
        provider().read_event(&payload, &header, now()),
        Err(BillingError::InvalidEventSignature)
    );
}

#[test]
fn a_timestamp_exactly_300_seconds_off_is_accepted_in_both_directions() {
    let payload = event("customer.updated", json!({}));

    for offset in [-300, 300] {
        let header = header_at(now().timestamp() + offset, &payload);
        assert!(
            provider().read_event(&payload, &header, now()).is_ok(),
            "{offset}"
        );
    }
}

#[test]
fn one_valid_signature_among_several_is_enough() {
    let payload = event("customer.updated", json!({}));
    let timestamp = now().timestamp();
    let good = mac_with(WEBHOOK_SECRET, timestamp, &payload);
    let bad = mac_with("whsec_rotated_out", timestamp, &payload);

    for header in [
        format!("t={timestamp},v1={bad},v1={good}"),
        format!("t={timestamp},v1={good},v1={bad}"),
        format!("v0={bad},t={timestamp},v1={good}"),
        format!("t={timestamp},v1=zz-not-hex,v1={good}"),
    ] {
        assert!(
            provider().read_event(&payload, &header, now()).is_ok(),
            "{header}"
        );
    }
}

#[test]
fn only_a_v0_signature_is_not_enough() {
    let payload = event("customer.updated", json!({}));
    let timestamp = now().timestamp();
    let header = format!(
        "t={timestamp},v0={}",
        mac_with(WEBHOOK_SECRET, timestamp, &payload)
    );

    assert_eq!(
        provider().read_event(&payload, &header, now()),
        Err(BillingError::InvalidEventSignature)
    );
}

#[test]
fn a_malformed_header_is_refused() {
    let payload = event("customer.updated", json!({}));
    let timestamp = now().timestamp();
    let good = mac_with(WEBHOOK_SECRET, timestamp, &payload);

    for header in [
        String::new(),
        "garbage".to_string(),
        format!("v1={good}"),
        format!("t={timestamp}"),
        format!("t=soon,v1={good}"),
        format!("t=,v1={good}"),
        format!("t={timestamp},v1="),
        format!("t={timestamp},v1=abc"),
        format!("t={timestamp},v1={good},oops"),
        format!("t={timestamp},,v1={good}"),
    ] {
        assert_eq!(
            provider().read_event(&payload, &header, now()),
            Err(BillingError::InvalidEventSignature),
            "{header:?}"
        );
    }
}

#[test]
fn a_signed_payload_that_is_not_an_event_is_refused() {
    for payload in [
        b"not json".to_vec(),
        b"{}".to_vec(),
        serde_json::to_vec(&json!({"id": "evt_1"})).unwrap(),
        serde_json::to_vec(&json!({"id": "evt 1", "type": "x"})).unwrap(),
        serde_json::to_vec(&json!({"id": "", "type": "x"})).unwrap(),
    ] {
        let header = header_at(now().timestamp(), &payload);
        assert_eq!(
            provider().read_event(&payload, &header, now()),
            Err(BillingError::InvalidEventSignature),
            "{}",
            String::from_utf8_lossy(&payload)
        );
    }
}

#[test]
fn a_succeeded_setup_intent_means_the_payment_method_is_ready() {
    let payload = event(
        "setup_intent.succeeded",
        json!({"id": "seti_1", "customer": "cus_9", "payment_method": "pm_9"}),
    );

    assert_eq!(
        read(&payload).unwrap(),
        ProviderEventKind::PaymentMethodReady {
            customer: ProviderCustomerId::parse("cus_9").unwrap(),
            payment_method: ProviderPaymentMethodId::parse("pm_9").unwrap(),
        }
    );
}

#[test]
fn a_setup_intent_missing_a_customer_or_a_method_is_ignored() {
    for object in [
        json!({"id": "seti_1", "payment_method": "pm_9"}),
        json!({"id": "seti_1", "customer": null, "payment_method": "pm_9"}),
        json!({"id": "seti_1", "customer": "cus_9"}),
        json!({"id": "seti_1", "customer": "cus_9", "payment_method": null}),
        json!({"id": "seti_1", "customer": {"id": "cus_9"}, "payment_method": "pm_9"}),
        json!({"id": "seti_1"}),
    ] {
        assert_eq!(
            read(&event("setup_intent.succeeded", object.clone())).unwrap(),
            ProviderEventKind::Ignored,
            "{object}"
        );
    }
}

#[test]
fn a_paid_invoice_maps_to_invoice_paid() {
    assert_eq!(
        read(&event("invoice.paid", json!({"id": "in_5"}))).unwrap(),
        ProviderEventKind::InvoicePaid {
            invoice: ProviderInvoiceId::parse("in_5").unwrap()
        }
    );
}

#[test]
fn a_failed_payment_maps_to_payment_failed() {
    assert_eq!(
        read(&event("invoice.payment_failed", json!({"id": "in_5"}))).unwrap(),
        ProviderEventKind::PaymentFailed {
            invoice: ProviderInvoiceId::parse("in_5").unwrap()
        }
    );
}

#[test]
fn an_uncollectible_invoice_maps_to_invoice_uncollectible() {
    assert_eq!(
        read(&event(
            "invoice.marked_uncollectible",
            json!({"id": "in_5"})
        ))
        .unwrap(),
        ProviderEventKind::InvoiceUncollectible {
            invoice: ProviderInvoiceId::parse("in_5").unwrap()
        }
    );
}

#[test]
fn an_invoice_event_without_an_invoice_id_is_refused() {
    for kind in [
        "invoice.paid",
        "invoice.payment_failed",
        "invoice.marked_uncollectible",
    ] {
        let payload = event(kind, json!({"object": "invoice"}));
        assert_eq!(
            read(&payload),
            Err(BillingError::InvalidEventSignature),
            "{kind}"
        );
    }
}

#[test]
fn any_other_event_is_ignored_and_keeps_its_id() {
    let payload = event("customer.updated", json!({"id": "cus_1"}));
    let header = header_at(now().timestamp(), &payload);

    let read = provider().read_event(&payload, &header, now()).unwrap();

    assert_eq!(read.kind, ProviderEventKind::Ignored);
    assert_eq!(read.id.as_str(), "evt_42");
}

#[test]
fn an_unknown_event_without_data_is_ignored() {
    let payload = serde_json::to_vec(&json!({"id": "evt_1", "type": "ping"})).unwrap();

    assert_eq!(read(&payload).unwrap(), ProviderEventKind::Ignored);
}
