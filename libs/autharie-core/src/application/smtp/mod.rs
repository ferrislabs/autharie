use autharie_auth::Identity;
use autharie_domain::smtp::{
    SmtpInput, SmtpOutcome, SmtpRealm,
    ports::{SmtpOutcomeService, SmtpPolicy, SmtpService},
    service::{SmtpOutcomeServiceImpl, SmtpServiceImpl},
};
use autharie_domain::{
    CoreError, deployments::DeploymentId, organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;

pub use autharie_domain::smtp::{
    FromEmail, FromName, RawSmtpInput, SMTP_ACTION_TYPE, SmtpActionPayload, SmtpActionSettings,
    SmtpConfig, SmtpEncryption, SmtpError, SmtpHost, SmtpPassword, SmtpPort, SmtpSettings,
    SmtpState, SmtpUsername, SmtpView, StoredSmtp, commands, entities, payload, ports, redacted,
    service, value_objects,
};

use crate::{
    AutharieService, infrastructure::credentials::EnvelopeVault,
    infrastructure::role::permissions_in, policy::PolicyContext,
};

struct InstanceRights<R> {
    permissions: R,
}

impl<R> SmtpPolicy for InstanceRights<R>
where
    R: PermissionProvider,
{
    async fn can_view_smtp(
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

    async fn can_manage_smtp(
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

impl SmtpService for AutharieService {
    #[transactional(deployment, smtp, webhook, action)]
    async fn get_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: SmtpRealm,
    ) -> Result<Option<SmtpView>, CoreError> {
        SmtpServiceImpl::new(
            deployment_repository,
            smtp_repository,
            webhook_repository,
            action_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .get_smtp(identity, organisation_id, deployment_id, realm)
        .await
    }

    #[transactional(deployment, smtp, webhook, action)]
    async fn save_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: SmtpInput,
    ) -> Result<SmtpView, CoreError> {
        SmtpServiceImpl::new(
            deployment_repository,
            smtp_repository,
            webhook_repository,
            action_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .save_smtp(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, smtp, webhook, action)]
    async fn remove_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: SmtpRealm,
    ) -> Result<(), CoreError> {
        SmtpServiceImpl::new(
            deployment_repository,
            smtp_repository,
            webhook_repository,
            action_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .remove_smtp(identity, organisation_id, deployment_id, realm)
        .await
    }

    #[transactional(deployment, smtp, webhook, action)]
    async fn branding_realms(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<String>, CoreError> {
        SmtpServiceImpl::new(
            deployment_repository,
            smtp_repository,
            webhook_repository,
            action_repository,
            EnvelopeVault::new(self.credential_keys())?,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .branding_realms(identity, organisation_id, deployment_id)
        .await
    }
}

impl SmtpOutcomeService for AutharieService {
    #[transactional(data_plane, deployment, smtp)]
    async fn report_smtp_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: SmtpOutcome,
    ) -> Result<bool, CoreError> {
        SmtpOutcomeServiceImpl::new(
            deployment_repository,
            data_plane_repository,
            smtp_repository,
        )
        .report_smtp_outcome(identity, deployment_id, outcome)
        .await
    }
}
