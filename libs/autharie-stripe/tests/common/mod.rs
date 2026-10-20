#![allow(dead_code)]

use autharie_domain::billing::{
    BillingAddress, BillingAddressInput, BillingDetails, BillingEmail, BillingName, BillingPeriod,
    Money, Siren, Statement, StatementId, StatementLine, StatementParts, StatementStatus,
    SubscriptionId, TierCode, VatNumber,
};
use autharie_domain::deployments::DeploymentId;
use autharie_domain::organisation::OrganisationId;
use autharie_stripe::{StripeConfig, StripePaymentProvider};
use chrono::{TimeZone, Utc};
use httpmock::MockServer;
use secrecy::SecretString;
use uuid::Uuid;

pub const SECRET_KEY: &str = "fixture-secret-key-0123456789";
pub const WEBHOOK_SECRET: &str = "fixture-webhook-secret-0123";

pub fn config(api_base: String) -> StripeConfig {
    StripeConfig {
        secret_key: SecretString::from(SECRET_KEY),
        webhook_secret: SecretString::from(WEBHOOK_SECRET),
        api_base,
    }
}

pub fn provider(server: &MockServer) -> StripePaymentProvider {
    StripePaymentProvider::new(config(server.base_url())).unwrap()
}

pub fn details() -> BillingDetails {
    BillingDetails {
        name: BillingName::parse("Acme SAS").unwrap(),
        email: BillingEmail::parse("billing@acme.test").unwrap(),
        address: BillingAddress::parse(BillingAddressInput {
            line1: "1 rue de la Paix",
            line2: None,
            postal_code: "75002",
            city: "Paris",
            country: "fr",
        })
        .unwrap(),
        vat_number: None,
        siren: None,
    }
}

pub fn full_details() -> BillingDetails {
    BillingDetails {
        address: BillingAddress::parse(BillingAddressInput {
            line1: "1 rue de la Paix",
            line2: Some("Bat. B"),
            postal_code: "75002",
            city: "Paris",
            country: "fr",
        })
        .unwrap(),
        vat_number: Some(VatNumber::parse("FR12345678901").unwrap()),
        siren: Some(Siren::parse("123456789").unwrap()),
        ..details()
    }
}

pub fn period() -> BillingPeriod {
    BillingPeriod::new(2026, 11).unwrap()
}

pub fn line(deployment: u128, subscription: u128, amount: i64) -> StatementLine {
    StatementLine {
        deployment_id: DeploymentId(Uuid::from_u128(deployment)),
        subscription_id: SubscriptionId(Uuid::from_u128(subscription)),
        tier: TierCode::parse("pro").unwrap(),
        monthly_price: Money::from_minor(4900),
        starts_at: Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
        ends_at: Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap(),
        covered_seconds: period().seconds(),
        amount: Money::from_minor(amount),
    }
}

pub fn statement_id() -> StatementId {
    StatementId(Uuid::from_u128(0x5747))
}

pub fn statement(lines: Vec<StatementLine>) -> Statement {
    let at = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();

    Statement::from(StatementParts {
        id: statement_id(),
        organisation_id: OrganisationId::from_uuid(Uuid::from_u128(0x0e6)),
        period: period(),
        lines,
        status: StatementStatus::Draft,
        invoice: None,
        created_at: at,
        updated_at: at,
    })
}

pub fn header<'a>(request: &'a httpmock::prelude::HttpMockRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .as_ref()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
