mod common;

use autharie_domain::CoreError;
use autharie_domain::billing::ports::PaymentProvider;
use autharie_domain::billing::{
    BillingDetails, BillingError, ProviderCustomerId, ProviderInvoiceId, ProviderPaymentMethodId,
};
use autharie_domain::organisation::OrganisationId;
use autharie_stripe::{DEFAULT_API_BASE, STRIPE_API_VERSION, StripeConfig, StripePaymentProvider};
use common::{
    SECRET_KEY, WEBHOOK_SECRET, config, details, full_details, header, line, provider, statement,
    statement_id,
};
use httpmock::prelude::*;
use secrecy::SecretString;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Mutex;
use uuid::Uuid;

fn customer() -> ProviderCustomerId {
    ProviderCustomerId::parse("cus_123").unwrap()
}

fn reason(error: CoreError) -> String {
    match error {
        CoreError::Billing(BillingError::Provider { reason }) => reason,
        other => panic!("expected a provider failure, got {other:?}"),
    }
}

#[test]
fn the_api_constants_are_the_documented_ones() {
    assert_eq!(STRIPE_API_VERSION, "2024-06-20");
    assert_eq!(DEFAULT_API_BASE, "https://api.stripe.com");
}

#[tokio::test]
async fn creating_a_customer_posts_the_details_under_a_per_organisation_key() {
    let server = MockServer::start();
    let organisation = OrganisationId::from_uuid(Uuid::from_u128(0x0e6));
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers")
            .header("Authorization", format!("Bearer {SECRET_KEY}"))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Idempotency-Key", format!("customer:{}", organisation.0))
            .x_www_form_urlencoded_tuple("name", "Acme SAS")
            .x_www_form_urlencoded_tuple("email", "billing@acme.test")
            .x_www_form_urlencoded_tuple("address[line1]", "1 rue de la Paix")
            .x_www_form_urlencoded_tuple("address[postal_code]", "75002")
            .x_www_form_urlencoded_tuple("address[city]", "Paris")
            .x_www_form_urlencoded_tuple("address[country]", "FR")
            .x_www_form_urlencoded_tuple("metadata[organisation_id]", organisation.0.to_string())
            .matches(|request| {
                let body = String::from_utf8_lossy(request.body.as_deref().unwrap_or_default())
                    .into_owned();
                !body.contains("line2") && !body.contains("vat_number") && !body.contains("siren")
            });
        then.status(200).json_body(json!({"id": "cus_123"}));
    });

    let created = provider(&server)
        .create_customer(&organisation, &details())
        .await
        .unwrap();

    assert_eq!(created.as_str(), "cus_123");
    mock.assert();
}

#[tokio::test]
async fn the_optional_details_are_sent_only_when_set() {
    let server = MockServer::start();
    let organisation = OrganisationId::from_uuid(Uuid::from_u128(0x0e6));
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers")
            .x_www_form_urlencoded_tuple("address[line2]", "Bat. B")
            .x_www_form_urlencoded_tuple("metadata[vat_number]", "FR12345678901")
            .x_www_form_urlencoded_tuple("metadata[siren]", "123456789");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });

    provider(&server)
        .create_customer(&organisation, &full_details())
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn updating_a_customer_keys_the_request_by_the_digest_of_its_body() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers/cus_123")
            .header("Authorization", format!("Bearer {SECRET_KEY}"))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .x_www_form_urlencoded_tuple("name", "Acme SAS")
            .x_www_form_urlencoded_tuple("address[city]", "Paris")
            .x_www_form_urlencoded_tuple("metadata[vat_number]", "")
            .x_www_form_urlencoded_tuple("metadata[siren]", "")
            .matches(|request| {
                let body = request.body.as_deref().unwrap_or_default();
                let digest = hex::encode(Sha256::digest(body));
                let expected = format!("customer:cus_123:update:{}", &digest[..32]);
                header(request, "Idempotency-Key") == Some(expected.as_str())
                    && !String::from_utf8_lossy(body).contains("organisation_id")
            });
        then.status(200).json_body(json!({"id": "cus_123"}));
    });

    provider(&server)
        .update_customer(&customer(), &details())
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn updating_a_customer_sends_the_tax_identifiers_it_holds() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers/cus_123")
            .x_www_form_urlencoded_tuple("metadata[vat_number]", "FR12345678901")
            .x_www_form_urlencoded_tuple("metadata[siren]", "123456789");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });

    provider(&server)
        .update_customer(&customer(), &full_details())
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn updating_a_customer_clears_a_tax_identifier_that_is_gone() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers/cus_123")
            .x_www_form_urlencoded_tuple("metadata[vat_number]", "")
            .x_www_form_urlencoded_tuple("metadata[siren]", "123456789");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });
    let details = BillingDetails {
        vat_number: None,
        ..full_details()
    };

    provider(&server)
        .update_customer(&customer(), &details)
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn updating_a_customer_clears_an_address_line_that_is_gone() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers/cus_123")
            .x_www_form_urlencoded_tuple("address[line2]", "");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });

    provider(&server)
        .update_customer(&customer(), &details())
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn updating_a_customer_with_other_details_changes_the_key() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/v1/customers/cus_123");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });
    let provider = provider(&server);

    provider
        .update_customer(&customer(), &details())
        .await
        .unwrap();
    provider
        .update_customer(&customer(), &full_details())
        .await
        .unwrap();

    mock.assert_hits(2);
}

#[tokio::test]
async fn a_setup_session_is_opened_under_a_fresh_key_each_time() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/checkout/sessions")
            .header("Authorization", format!("Bearer {SECRET_KEY}"))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .x_www_form_urlencoded_tuple("mode", "setup")
            .x_www_form_urlencoded_tuple("customer", "cus_123")
            .x_www_form_urlencoded_tuple("currency", "eur")
            .x_www_form_urlencoded_tuple("success_url", "https://app.test/billing?done=1")
            .x_www_form_urlencoded_tuple("cancel_url", "https://app.test/billing?done=1")
            .matches(|request| {
                header(request, "Idempotency-Key")
                    .and_then(|key| key.strip_prefix("checkout:cus_123:"))
                    .and_then(|rest| Uuid::parse_str(rest).ok())
                    .is_some_and(|uuid| uuid.get_version_num() == 4)
            });
        then.status(200)
            .json_body(json!({"id": "cs_1", "url": "https://checkout.stripe.test/c/pay/cs_1"}));
    });
    let provider = provider(&server);

    let url = provider
        .payment_method_session(&customer(), "https://app.test/billing?done=1")
        .await
        .unwrap();
    provider
        .payment_method_session(&customer(), "https://app.test/billing?done=1")
        .await
        .unwrap();

    assert_eq!(url, "https://checkout.stripe.test/c/pay/cs_1");
    mock.assert_hits(2);
}

static SESSION_KEYS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

#[tokio::test]
async fn two_setup_sessions_never_share_a_key() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/checkout/sessions")
            .matches(|request| {
                let key = header(request, "Idempotency-Key").unwrap_or_default();
                SESSION_KEYS.lock().unwrap().insert(key.to_string());
                true
            });
        then.status(200)
            .json_body(json!({"url": "https://c.test/1"}));
    });
    let provider = provider(&server);

    for _ in 0..2 {
        provider
            .payment_method_session(&customer(), "https://app.test/a")
            .await
            .unwrap();
    }

    mock.assert_hits(2);
    assert_eq!(SESSION_KEYS.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_session_without_a_url_is_a_provider_failure() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/checkout/sessions");
        then.status(200).json_body(json!({"id": "cs_1"}));
    });

    let error = provider(&server)
        .payment_method_session(&customer(), "https://app.test/a")
        .await
        .unwrap_err();

    assert!(reason(error).contains("/v1/checkout/sessions"));
}

#[tokio::test]
async fn the_default_payment_method_is_set_under_a_key_naming_both_ids() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/customers/cus_123")
            .header("Authorization", format!("Bearer {SECRET_KEY}"))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header(
                "Idempotency-Key",
                "customer:cus_123:default-payment-method:pm_9",
            )
            .x_www_form_urlencoded_tuple("invoice_settings[default_payment_method]", "pm_9");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });

    provider(&server)
        .set_default_payment_method(
            &customer(),
            &ProviderPaymentMethodId::parse("pm_9").unwrap(),
        )
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn an_invoice_is_built_item_by_item_then_finalized_under_stable_keys() {
    let server = MockServer::start();
    let auth = format!("Bearer {SECRET_KEY}");
    let statement = statement(vec![line(0xd1, 0x51, 4900), line(0xd2, 0x52, 1234)]);
    let sid = statement_id();

    let invoice = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/invoices")
            .header("Authorization", auth.clone())
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", format!("statement:{sid}"))
            .x_www_form_urlencoded_tuple("customer", "cus_123")
            .x_www_form_urlencoded_tuple("collection_method", "charge_automatically")
            .x_www_form_urlencoded_tuple("currency", "eur")
            .x_www_form_urlencoded_tuple("auto_advance", "true")
            .x_www_form_urlencoded_tuple("pending_invoice_items_behavior", "exclude")
            .x_www_form_urlencoded_tuple("metadata[statement_id]", sid.to_string())
            .x_www_form_urlencoded_tuple("metadata[period]", "2026-11")
            .x_www_form_urlencoded_tuple("description", "Autharie 2026-11");
        then.status(200).json_body(json!({"id": "in_77"}));
    });
    let first_item = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/invoiceitems")
            .header("Authorization", auth.clone())
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", format!("statement:{sid}:line:0"))
            .x_www_form_urlencoded_tuple("customer", "cus_123")
            .x_www_form_urlencoded_tuple("invoice", "in_77")
            .x_www_form_urlencoded_tuple("currency", "eur")
            .x_www_form_urlencoded_tuple("amount", "4900")
            .x_www_form_urlencoded_tuple(
                "description",
                format!(
                    "pro for deployment {}, 2026-11-01T00:00:00+00:00 to 2026-12-01T00:00:00+00:00",
                    Uuid::from_u128(0xd1)
                ),
            )
            .x_www_form_urlencoded_tuple("period[start]", "1793491200")
            .x_www_form_urlencoded_tuple("period[end]", "1796083200")
            .x_www_form_urlencoded_tuple(
                "metadata[deployment_id]",
                Uuid::from_u128(0xd1).to_string(),
            )
            .x_www_form_urlencoded_tuple(
                "metadata[subscription_id]",
                Uuid::from_u128(0x51).to_string(),
            );
        then.status(200).json_body(json!({"id": "ii_1"}));
    });
    let second_item = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/invoiceitems")
            .header("Authorization", auth.clone())
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", format!("statement:{sid}:line:1"))
            .x_www_form_urlencoded_tuple("invoice", "in_77")
            .x_www_form_urlencoded_tuple("amount", "1234")
            .x_www_form_urlencoded_tuple(
                "metadata[deployment_id]",
                Uuid::from_u128(0xd2).to_string(),
            )
            .x_www_form_urlencoded_tuple(
                "metadata[subscription_id]",
                Uuid::from_u128(0x52).to_string(),
            );
        then.status(200).json_body(json!({"id": "ii_2"}));
    });
    let finalize = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/invoices/in_77/finalize")
            .header("Authorization", auth.clone())
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", format!("statement:{sid}:finalize"))
            .x_www_form_urlencoded_tuple("auto_advance", "true");
        then.status(200).json_body(json!({
            "id": "in_77",
            "hosted_invoice_url": "https://invoice.stripe.test/i/in_77"
        }));
    });

    let issued = provider(&server)
        .issue_invoice(&customer(), &statement)
        .await
        .unwrap();

    assert_eq!(issued.id, ProviderInvoiceId::parse("in_77").unwrap());
    assert_eq!(issued.hosted_url, "https://invoice.stripe.test/i/in_77");
    invoice.assert();
    first_item.assert();
    second_item.assert();
    finalize.assert();
}

#[tokio::test]
async fn a_refused_invoice_item_stops_the_invoice_before_it_is_finalized() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/invoices");
        then.status(200).json_body(json!({"id": "in_77"}));
    });
    let first_item = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/invoiceitems")
            .x_www_form_urlencoded_tuple("amount", "4900");
        then.status(200).json_body(json!({"id": "ii_1"}));
    });
    let refused_item = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/invoiceitems")
            .x_www_form_urlencoded_tuple("amount", "1234");
        then.status(400).json_body(json!({
            "error": {
                "type": "invalid_request_error",
                "code": "amount_too_large",
                "message": "customer cus_123 cannot be charged"
            }
        }));
    });
    let finalize = server.mock(|when, then| {
        when.method(POST).path("/v1/invoices/in_77/finalize");
        then.status(200).json_body(json!({
            "id": "in_77",
            "hosted_invoice_url": "https://invoice.stripe.test/i/in_77"
        }));
    });
    let statement = statement(vec![line(0xd1, 0x51, 4900), line(0xd2, 0x52, 1234)]);

    let error = provider(&server)
        .issue_invoice(&customer(), &statement)
        .await
        .unwrap_err();

    let reason = reason(error);
    assert!(reason.contains("400"), "{reason}");
    assert!(reason.contains("/v1/invoiceitems"), "{reason}");
    assert!(reason.contains("invalid_request_error"), "{reason}");
    assert!(reason.contains("amount_too_large"), "{reason}");
    assert!(!reason.contains("cus_123"), "{reason}");
    assert!(!reason.contains("cannot be charged"), "{reason}");
    first_item.assert();
    refused_item.assert();
    finalize.assert_hits(0);
}

#[tokio::test]
async fn a_finalized_invoice_without_a_hosted_url_is_a_provider_failure() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/invoices");
        then.status(200).json_body(json!({"id": "in_77"}));
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/invoiceitems");
        then.status(200).json_body(json!({"id": "ii_1"}));
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/invoices/in_77/finalize");
        then.status(200)
            .json_body(json!({"id": "in_77", "hosted_invoice_url": null}));
    });

    let error = provider(&server)
        .issue_invoice(&customer(), &statement(vec![line(0xd1, 0x51, 4900)]))
        .await
        .unwrap_err();

    assert!(reason(error).contains("/v1/invoices/{invoice}/finalize"));
}

#[tokio::test]
async fn a_customer_id_cannot_climb_out_of_its_path() {
    let server = MockServer::start();
    let reached = server.mock(|when, then| {
        when.method(POST).path("/v1/customers/cus_123");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });
    let hostile = ProviderCustomerId::parse("cus_123/../../checkout/sessions?x=1#y").unwrap();

    let outcome = provider(&server)
        .update_customer(&hostile, &details())
        .await;

    assert!(outcome.is_err());
    reached.assert_hits(0);
}

#[tokio::test]
async fn a_rejected_key_never_reaches_the_error() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/customers");
        then.status(401).json_body(json!({
            "error": {
                "type": "invalid_request_error",
                "message": format!("Invalid API Key provided: {SECRET_KEY}")
            }
        }));
    });

    let error = provider(&server)
        .create_customer(&OrganisationId::new(), &details())
        .await
        .unwrap_err();

    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains(SECRET_KEY), "{rendered}");
    assert!(!rendered.contains("sk_test"), "{rendered}");
    assert!(rendered.contains("401"), "{rendered}");
    assert!(rendered.contains("/v1/customers"), "{rendered}");
    assert!(rendered.contains("invalid_request_error"), "{rendered}");
}

#[tokio::test]
async fn an_error_body_that_is_not_json_still_gives_a_status() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/customers");
        then.status(502).body(format!("<html>{SECRET_KEY}</html>"));
    });

    let error = provider(&server)
        .create_customer(&OrganisationId::new(), &details())
        .await
        .unwrap_err();

    let rendered = reason(error);
    assert!(rendered.contains("502"), "{rendered}");
    assert!(!rendered.contains(SECRET_KEY), "{rendered}");
}

#[tokio::test]
async fn an_error_type_that_is_not_a_token_is_left_out() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/customers");
        then.status(400).json_body(json!({
            "error": {"type": format!("see {SECRET_KEY}"), "code": "card_declined"}
        }));
    });

    let error = provider(&server)
        .create_customer(&OrganisationId::new(), &details())
        .await
        .unwrap_err();

    let rendered = reason(error);
    assert!(!rendered.contains(SECRET_KEY), "{rendered}");
    assert!(rendered.contains("card_declined"), "{rendered}");
}

#[tokio::test]
async fn an_unreachable_endpoint_is_a_provider_failure_without_the_key() {
    let provider = StripePaymentProvider::new(config("http://127.0.0.1:1".to_string())).unwrap();

    let error = provider
        .create_customer(&OrganisationId::new(), &details())
        .await
        .unwrap_err();

    let rendered = format!("{error} {error:?}");
    assert!(matches!(
        error,
        CoreError::Billing(BillingError::Provider { .. })
    ));
    assert!(!rendered.contains(SECRET_KEY), "{rendered}");
    assert!(!rendered.contains("127.0.0.1"), "{rendered}");
}

#[tokio::test]
async fn a_trailing_slash_on_the_base_is_ignored() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/v1/customers");
        then.status(200).json_body(json!({"id": "cus_123"}));
    });
    let provider = StripePaymentProvider::new(config(format!("{}/", server.base_url()))).unwrap();

    provider
        .create_customer(&OrganisationId::new(), &details())
        .await
        .unwrap();

    mock.assert();
}

#[tokio::test]
async fn an_answer_that_is_not_the_expected_shape_is_a_provider_failure() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/customers");
        then.status(200).body("not json");
    });

    let error = provider(&server)
        .create_customer(&OrganisationId::new(), &details())
        .await
        .unwrap_err();

    assert!(reason(error).contains("/v1/customers"));
}

#[test]
fn an_empty_or_blank_secret_key_means_billing_is_not_configured() {
    for blank in ["", " ", "\t\n  "] {
        let config = StripeConfig {
            secret_key: SecretString::from(blank),
            ..config("https://api.stripe.test".to_string())
        };

        assert_eq!(
            StripePaymentProvider::new(config).unwrap_err(),
            BillingError::NotConfigured,
            "{blank:?}"
        );
    }
}

#[test]
fn an_empty_or_blank_webhook_secret_means_billing_is_not_configured() {
    for blank in ["", " ", "\t\n  "] {
        let config = StripeConfig {
            webhook_secret: SecretString::from(blank),
            ..config("https://api.stripe.test".to_string())
        };

        assert_eq!(
            StripePaymentProvider::new(config).unwrap_err(),
            BillingError::NotConfigured,
            "{blank:?}"
        );
    }
}

#[test]
fn the_debug_of_the_config_and_of_the_provider_hides_both_secrets() {
    let config = StripeConfig {
        secret_key: SecretString::from(SECRET_KEY),
        webhook_secret: SecretString::from(WEBHOOK_SECRET),
        api_base: "https://api.stripe.test".to_string(),
    };

    let shown = format!("{config:?}");
    assert!(!shown.contains(SECRET_KEY), "{shown}");
    assert!(!shown.contains(WEBHOOK_SECRET), "{shown}");
    assert!(shown.contains("https://api.stripe.test"), "{shown}");
    assert!(shown.contains("[redacted]"), "{shown}");

    let provider = StripePaymentProvider::new(config).unwrap();
    let shown = format!("{provider:?}");
    assert!(!shown.contains(SECRET_KEY), "{shown}");
    assert!(!shown.contains(WEBHOOK_SECRET), "{shown}");
    assert!(!shown.contains("sk_test"), "{shown}");
}
