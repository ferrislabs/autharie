pub mod account;
pub mod accounts;
pub mod closing;
pub mod collection;
pub mod events;
pub mod lifecycle;
pub mod money;
pub mod period;
pub mod ports;
pub mod rating;
pub mod sending;
pub mod statement;
pub mod subscription;
pub mod suspension;
pub mod tier;

pub use account::{
    BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail, BillingName,
    CountryCode, PaymentMethodStatus, ProviderCustomerId, Siren, VatNumber, require_complete,
};
pub use events::{ProviderEvent, ProviderEventId, ProviderEventKind, ProviderPaymentMethodId};
pub use money::Money;
pub use period::BillingPeriod;
pub use rating::{FirstInvoice, estimate, first_invoice, rate};
pub use statement::{
    ProviderInvoice, ProviderInvoiceId, Statement, StatementId, StatementLine, StatementParts,
    StatementStatus,
};
pub use subscription::{
    Segment, Subscription, SubscriptionId, SubscriptionParts, SubscriptionTerms,
};
pub use suspension::{BillingSuspension, BillingSuspensionId};
pub use tier::{Hosting, Tier, TierCode};

use thiserror::Error;
use uuid::Uuid;

use crate::{
    dataplane::value_objects::{DataPlaneMode, DeploymentResources},
    offers::DatabasePlacement,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BillingError {
    #[error("'{value}' is not a billing period, expected YYYY-MM")]
    InvalidPeriod { value: String },

    #[error("'{value}' is not a tier code")]
    InvalidTierCode { value: String },

    #[error("'{value}' is not a hosting")]
    InvalidHosting { value: String },

    #[error("'{value}' is not a payment method status")]
    InvalidPaymentMethodStatus { value: String },

    #[error("'{value}' is not a statement status")]
    InvalidStatementStatus { value: String },

    #[error("invalid {field}: {reason}")]
    InvalidBillingDetails {
        field: &'static str,
        reason: &'static str,
    },

    #[error("'{value}' is not a valid provider reference")]
    InvalidProviderReference { value: String },

    #[error("a tier is required: pick one of the catalogue")]
    TierRequired,
    #[error("the organisation has no billing account: give its name, e-mail and address first")]
    NoBillingAccount,
    #[error("there is no statement for {period}")]
    StatementNotFound { period: String },
    #[error("tier '{code}' does not exist")]
    TierNotFound { code: String },

    #[error("tier '{code}' cannot be selected")]
    TierNotSelectable { code: String },

    #[error("tier '{code}' does not allow the {engine} engine")]
    TierDisallowsEngine { code: String, engine: String },

    #[error("tier '{code}' does not allow {hosting} hosting")]
    TierDisallowsHosting { code: String, hosting: String },

    #[error(
        "a paid tier needs a complete billing account: a name, an e-mail, an address and a valid payment method"
    )]
    AccountIncomplete,

    #[error("subscription {id} is already closed")]
    SubscriptionClosed { id: Uuid },

    #[error("a subscription cannot end before it starts")]
    SubscriptionEndsBeforeStart,

    #[error("deployment {deployment} has no open subscription")]
    NoOpenSubscription { deployment: Uuid },

    #[error("statement {id} cannot go from {from} to {to}")]
    InvalidStatementTransition { id: Uuid, from: String, to: String },

    #[error("the signature of the provider event is invalid")]
    InvalidEventSignature,

    #[error("invoice {invoice} is not recorded yet, the event will be retried")]
    InvoiceNotRecordedYet { invoice: String },

    #[error("payment provider failure: {reason}")]
    Provider { reason: String },

    #[error("billing is not configured")]
    NotConfigured,

    #[error("moving a deployment from a {from} database to a {to} one is not available yet")]
    DatabasePlacementChange {
        from: DatabasePlacement,
        to: DatabasePlacement,
    },

    #[error("moving a deployment from a {from} data plane to a {to} one is not available yet")]
    DataPlaneModeChange {
        from: DataPlaneMode,
        to: DataPlaneMode,
    },

    #[error("resizing a {hosting} deployment is not available yet")]
    ResizeNotAvailable { hosting: Hosting },

    #[error(
        "the data plane has no room for the size of this tier: {} millicores of CPU, {} MiB of memory and {} GiB of storage",
        .wanted.cpu_millis, .wanted.memory_mib, .wanted.storage_gib
    )]
    NoRoomToResize { wanted: DeploymentResources },
}
