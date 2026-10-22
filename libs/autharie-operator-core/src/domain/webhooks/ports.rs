use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, WebhookRequestStatus, WebhookStatus,
};

use super::{RemoteWebhook, WebhookError, WebhookWrite};
use crate::domain::OperatorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookOutcome {
    Settled,
    /// Something failed or is waiting; look again soon.
    Retry,
}

pub trait IdentityInstanceWebhooks: Send + Sync {
    fn reconcile(&self, instance: &IdentityInstance)
    -> impl Future<Output = WebhookOutcome> + Send;
}

/// FerrisKey's admin API, as far as webhooks go. Every call is made as the
/// instance's admin, and realms are FerrisKey's, by name.
pub trait FerriskeyWebhookPort: Send + Sync {
    fn list_realms(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<Vec<String>, WebhookError>> + Send;

    /// Every webhook of the realm named like a managed one. Anything else the
    /// realm holds is not this port's business.
    fn list_managed_webhooks(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> impl Future<Output = Result<Vec<RemoteWebhook>, WebhookError>> + Send;

    fn create_webhook(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        write: &WebhookWrite,
    ) -> impl Future<Output = Result<RemoteWebhook, WebhookError>> + Send;

    fn update_webhook(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
        write: &WebhookWrite,
    ) -> impl Future<Output = Result<(), WebhookError>> + Send;

    /// A webhook that is already gone is deleted.
    fn delete_webhook(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
    ) -> impl Future<Output = Result<(), WebhookError>> + Send;

    /// The new signing secret. FerrisKey shows it here and nowhere else.
    fn rotate_secret(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
    ) -> impl Future<Output = Result<String, WebhookError>> + Send;

    fn retry_delivery(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
        delivery_id: &str,
    ) -> impl Future<Output = Result<(), WebhookError>> + Send;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretsSnapshot {
    /// `<webhook id>.<header name>` to the value. Written by Genesis.
    pub headers: BTreeMap<String, String>,
    /// Webhook id to the fingerprint of the header values last sent.
    pub applied: BTreeMap<String, String>,
    /// Webhook ids whose signing secret is stored. The secrets themselves
    /// stay in the cluster.
    pub secret_ids: BTreeSet<String>,
}

/// The Secrets in the instance's namespace the operator reads and writes.
pub trait WebhookSecretsPort: Send + Sync {
    fn load(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<SecretsSnapshot, WebhookError>> + Send;

    fn put_secret(
        &self,
        instance: &IdentityInstance,
        webhook_id: &str,
        secret: &str,
    ) -> impl Future<Output = Result<(), WebhookError>> + Send;

    fn put_applied(
        &self,
        instance: &IdentityInstance,
        webhook_id: &str,
        fingerprint: &str,
    ) -> impl Future<Output = Result<(), WebhookError>> + Send;

    fn forget(
        &self,
        instance: &IdentityInstance,
        webhook_ids: &[String],
    ) -> impl Future<Output = Result<(), WebhookError>> + Send;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebhookStatusReport {
    pub webhooks: Vec<WebhookStatus>,
    pub realms: Vec<String>,
    pub webhook_requests: Vec<WebhookRequestStatus>,
}

/// Writes only the webhook fields of the status, so it cannot undo what the
/// instance reconcile wrote from an older copy.
pub trait WebhookStatusRepository: Send + Sync {
    fn patch_webhook_status(
        &self,
        instance: &IdentityInstance,
        report: &WebhookStatusReport,
    ) -> impl Future<Output = Result<(), OperatorError>> + Send;
}
