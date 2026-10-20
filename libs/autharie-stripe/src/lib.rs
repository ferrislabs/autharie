mod client;
mod form;
mod invoice;
mod webhook;
mod wire;

use std::fmt;

use autharie_domain::{
    CoreError,
    billing::{
        BillingDetails, BillingError, ProviderCustomerId, ProviderEvent, ProviderInvoice,
        ProviderPaymentMethodId, Siren, Statement, VatNumber, ports::PaymentProvider,
    },
    organisation::OrganisationId,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use client::{Client, Key, malformed};
use form::Form;
use wire::{Created, Session};

pub const STRIPE_API_VERSION: &str = "2024-06-20";
pub const DEFAULT_API_BASE: &str = "https://api.stripe.com";

const CURRENCY: &str = "eur";
const CUSTOMERS: &str = "/v1/customers";
const CUSTOMER: &str = "/v1/customers/{customer}";
const CHECKOUT_SESSIONS: &str = "/v1/checkout/sessions";

pub struct StripeConfig {
    pub secret_key: SecretString,
    pub webhook_secret: SecretString,
    pub api_base: String,
}

impl fmt::Debug for StripeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StripeConfig")
            .field("secret_key", &format_args!("[redacted]"))
            .field("webhook_secret", &format_args!("[redacted]"))
            .field("api_base", &self.api_base)
            .finish()
    }
}

pub struct StripePaymentProvider {
    client: Client,
    webhook_secret: SecretString,
}

impl fmt::Debug for StripePaymentProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StripePaymentProvider")
            .field("api_base", &self.client.base())
            .field("secret_key", &format_args!("[redacted]"))
            .field("webhook_secret", &format_args!("[redacted]"))
            .finish()
    }
}

impl StripePaymentProvider {
    pub fn new(config: StripeConfig) -> Result<Self, BillingError> {
        let StripeConfig {
            secret_key,
            webhook_secret,
            api_base,
        } = config;

        if is_blank(&secret_key) || is_blank(&webhook_secret) {
            return Err(BillingError::NotConfigured);
        }

        Ok(Self {
            client: Client::new(secret_key, &api_base)?,
            webhook_secret,
        })
    }
}

fn is_blank(secret: &SecretString) -> bool {
    secret.expose_secret().trim().is_empty()
}

fn identity_form(details: &BillingDetails) -> Form<'_> {
    let address = &details.address;

    Form::new()
        .field("name", details.name.as_str())
        .field("email", details.email.as_str())
        .field("address[line1]", address.line1())
        .field("address[postal_code]", address.postal_code())
        .field("address[city]", address.city())
        .field("address[country]", address.country().as_str())
}

fn vat_number(details: &BillingDetails) -> Option<&str> {
    details.vat_number.as_ref().map(VatNumber::as_str)
}

fn siren(details: &BillingDetails) -> Option<&str> {
    details.siren.as_ref().map(Siren::as_str)
}

fn new_customer_form<'a>(details: &'a BillingDetails, organisation: &'a str) -> Form<'a> {
    identity_form(details)
        .optional("address[line2]", details.address.line2())
        .field("metadata[organisation_id]", organisation)
        .optional("metadata[vat_number]", vat_number(details))
        .optional("metadata[siren]", siren(details))
}

fn customer_update_form(details: &BillingDetails) -> Form<'_> {
    identity_form(details)
        .field(
            "address[line2]",
            details.address.line2().unwrap_or_default(),
        )
        .field(
            "metadata[vat_number]",
            vat_number(details).unwrap_or_default(),
        )
        .field("metadata[siren]", siren(details).unwrap_or_default())
}

impl PaymentProvider for StripePaymentProvider {
    async fn create_customer(
        &self,
        organisation_id: &OrganisationId,
        details: &BillingDetails,
    ) -> Result<ProviderCustomerId, CoreError> {
        let organisation = organisation_id.0.to_string();
        let form = new_customer_form(details, &organisation);

        let created: Created = self
            .client
            .post(
                CUSTOMERS,
                &[],
                &form,
                Key::Fixed(format!("customer:{organisation}")),
            )
            .await?;

        Ok(ProviderCustomerId::parse(&created.id).map_err(|_| malformed(CUSTOMERS))?)
    }

    async fn update_customer(
        &self,
        customer: &ProviderCustomerId,
        details: &BillingDetails,
    ) -> Result<(), CoreError> {
        let form = customer_update_form(details);

        self.client
            .post::<Created>(
                CUSTOMER,
                &[customer.as_str()],
                &form,
                Key::BodyDigest(format!("customer:{}:update", customer.as_str())),
            )
            .await?;

        Ok(())
    }

    async fn payment_method_session(
        &self,
        customer: &ProviderCustomerId,
        return_url: &str,
    ) -> Result<String, CoreError> {
        let form = Form::new()
            .field("mode", "setup")
            .field("customer", customer.as_str())
            .field("currency", CURRENCY)
            .field("success_url", return_url)
            .field("cancel_url", return_url);

        let session: Session = self
            .client
            .post(
                CHECKOUT_SESSIONS,
                &[],
                &form,
                Key::Fixed(format!("checkout:{}:{}", customer.as_str(), Uuid::new_v4())),
            )
            .await?;

        Ok(session.url.ok_or_else(|| malformed(CHECKOUT_SESSIONS))?)
    }

    async fn set_default_payment_method(
        &self,
        customer: &ProviderCustomerId,
        payment_method: &ProviderPaymentMethodId,
    ) -> Result<(), CoreError> {
        let form = Form::new().field(
            "invoice_settings[default_payment_method]",
            payment_method.as_str(),
        );

        self.client
            .post::<Created>(
                CUSTOMER,
                &[customer.as_str()],
                &form,
                Key::Fixed(format!(
                    "customer:{}:default-payment-method:{}",
                    customer.as_str(),
                    payment_method.as_str()
                )),
            )
            .await?;

        Ok(())
    }

    async fn issue_invoice(
        &self,
        customer: &ProviderCustomerId,
        statement: &Statement,
    ) -> Result<ProviderInvoice, CoreError> {
        Ok(invoice::issue(&self.client, customer, statement).await?)
    }

    fn read_event(
        &self,
        payload: &[u8],
        signature: &str,
        now: DateTime<Utc>,
    ) -> Result<ProviderEvent, BillingError> {
        webhook::read(&self.webhook_secret, payload, signature, now)
    }
}
