use autharie_auth::Identity;
use autharie_domain::webhooks::{
    commands::{WebhookInput, WebhooksOutcome},
    delivery::{DeliveryQuery, WebhookDeliveryIndex},
    ports::{WebhookOutcomeService, WebhookPolicy, WebhookService},
    service::{WebhookOutcomeServiceImpl, WebhookServiceImpl},
};
use autharie_domain::{
    CoreError, deployments::DeploymentId, organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;

pub use autharie_domain::webhooks::{
    DeliveryStatus, HeaderInput, HeaderName, RequestKind, RequestPhase, RevealedSecret,
    SecretRecord, StoredWebhook, Webhook, WebhookDelivery, WebhookEndpoint, WebhookError,
    WebhookEvent, WebhookEventInfo, WebhookId, WebhookName, WebhookRealm, WebhookRequest,
    WebhookState, WebhookSync, WebhookView, commands, delivery, entities, known_events, ports,
    redacted, service, value_objects,
};

use crate::{
    AutharieService, infrastructure::credentials::EnvelopeVault,
    infrastructure::role::permissions_in, policy::PolicyContext,
};

struct InstanceRights<R> {
    permissions: R,
}

impl<R> WebhookPolicy for InstanceRights<R>
where
    R: PermissionProvider,
{
    async fn can_view_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        PolicyContext::new(permissions).require_permission(Permissions::VIEW_INSTANCES)
    }

    async fn can_manage_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        PolicyContext::new(permissions).require_permission(Permissions::MANAGE_INSTANCES)
    }
}

impl WebhookService for AutharieService {
    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn list_webhooks(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<WebhookView>, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .list_webhooks(identity, organisation_id, deployment_id)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn get_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<WebhookView, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .get_webhook(identity, organisation_id, deployment_id, webhook_id)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn create_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: WebhookInput,
    ) -> Result<WebhookView, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .create_webhook(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn update_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
        input: WebhookInput,
    ) -> Result<WebhookView, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .update_webhook(identity, organisation_id, deployment_id, webhook_id, input)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn delete_webhook(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<(), CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .delete_webhook(identity, organisation_id, deployment_id, webhook_id)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn rotate_webhook_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<WebhookRequest, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .rotate_webhook_secret(identity, organisation_id, deployment_id, webhook_id)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn reveal_webhook_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
    ) -> Result<RevealedSecret, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .reveal_webhook_secret(identity, organisation_id, deployment_id, webhook_id)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn retry_webhook_delivery(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        webhook_id: WebhookId,
        delivery_id: String,
    ) -> Result<WebhookRequest, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .retry_webhook_delivery(
            identity,
            organisation_id,
            deployment_id,
            webhook_id,
            delivery_id,
        )
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn webhook_realms(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<String>, CoreError> {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .webhook_realms(identity, organisation_id, deployment_id)
        .await
    }

    #[transactional(deployment, organisation, webhook, action, audit, user)]
    async fn webhook_deliveries<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        query: DeliveryQuery,
        index: I,
    ) -> Result<Vec<WebhookDelivery>, CoreError>
    where
        I: WebhookDeliveryIndex,
    {
        WebhookServiceImpl::new(
            deployment_repository,
            organisation_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .webhook_deliveries(identity, organisation_id, deployment_id, query, index)
        .await
    }
}

impl WebhookOutcomeService for AutharieService {
    #[transactional(data_plane, deployment, webhook)]
    async fn report_webhooks_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: WebhooksOutcome,
    ) -> Result<bool, CoreError> {
        WebhookOutcomeServiceImpl::new(
            deployment_repository,
            data_plane_repository,
            webhook_repository,
            EnvelopeVault::new(self.credential_keys())?,
        )
        .report_webhooks_outcome(identity, deployment_id, outcome)
        .await
    }
}
