use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    CoreError,
    dataplane::{credential::SecretString, credential_repository::SealedSecret},
    deployments::DeploymentId,
    organisation::OrganisationId,
    webhooks::{
        commands::{WebhookInput, WebhooksOutcome},
        delivery::{DeliveryQuery, WebhookDelivery, WebhookDeliveryIndex},
        entities::{
            RequestPhase, RevealedSecret, SecretRecord, StoredWebhook, WebhookRequest,
            WebhookState, WebhookSync, WebhookView,
        },
        value_objects::WebhookId,
    },
};

pub trait WebhookPolicy: Send + Sync {
    fn can_view_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn can_manage_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait SecretVault: Send + Sync {
    fn seal(
        &self,
        context: &str,
        plaintext: &SecretString,
    ) -> impl Future<Output = Result<SealedSecret, CoreError>> + Send;

    fn open(
        &self,
        context: &str,
        sealed: &SealedSecret,
    ) -> impl Future<Output = Result<SecretString, CoreError>> + Send;
}

pub trait WebhookRepository: Send + Sync {
    fn next_revision(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;

    fn sync_state(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<WebhookSync, CoreError>> + Send;

    fn record_applied(
        &self,
        deployment_id: DeploymentId,
        revision: u64,
        realms: &[String],
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn list(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<StoredWebhook>, CoreError>> + Send;

    fn get(
        &self,
        deployment_id: DeploymentId,
        id: WebhookId,
    ) -> impl Future<Output = Result<Option<StoredWebhook>, CoreError>> + Send;

    fn insert(&self, webhook: &StoredWebhook)
    -> impl Future<Output = Result<(), CoreError>> + Send;

    fn update(
        &self,
        webhook: &StoredWebhook,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn delete(
        &self,
        deployment_id: DeploymentId,
        id: WebhookId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn set_state(
        &self,
        id: WebhookId,
        state: &WebhookState,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn secret(
        &self,
        id: WebhookId,
    ) -> impl Future<Output = Result<Option<SecretRecord>, CoreError>> + Send;

    fn store_secret(
        &self,
        id: WebhookId,
        secret: &SecretRecord,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn insert_request(
        &self,
        request: &WebhookRequest,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn finish_request(
        &self,
        id: Uuid,
        phase: RequestPhase,
        message: Option<&str>,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn recent_requests(
        &self,
        deployment_id: DeploymentId,
        per_webhook: usize,
    ) -> impl Future<Output = Result<Vec<WebhookRequest>, CoreError>> + Send;
}

pub trait WebhookService: Send + Sync {
    fn list_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<WebhookView>, CoreError>> + Send;

    fn get_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> impl Future<Output = Result<WebhookView, CoreError>> + Send;

    fn create_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: WebhookInput,
    ) -> impl Future<Output = Result<WebhookView, CoreError>> + Send;

    fn update_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
        input: WebhookInput,
    ) -> impl Future<Output = Result<WebhookView, CoreError>> + Send;

    fn delete_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn rotate_webhook_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> impl Future<Output = Result<WebhookRequest, CoreError>> + Send;

    fn reveal_webhook_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> impl Future<Output = Result<RevealedSecret, CoreError>> + Send;

    fn retry_webhook_delivery(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
        delivery_id: String,
    ) -> impl Future<Output = Result<WebhookRequest, CoreError>> + Send;

    fn webhook_realms(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<String>, CoreError>> + Send;

    fn webhook_deliveries<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        query: DeliveryQuery,
        index: I,
    ) -> impl Future<Output = Result<Vec<WebhookDelivery>, CoreError>> + Send
    where
        I: WebhookDeliveryIndex;
}

pub trait WebhookOutcomeService: Send + Sync {
    fn report_webhooks_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: WebhooksOutcome,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
