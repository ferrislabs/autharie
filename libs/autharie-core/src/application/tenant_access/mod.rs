use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    cells::{
        AdministratorCredentialError, AdministratorVault, InitialAdministrator,
        TenantAdministratorRepository,
    },
    dataplane::herald_identity::speaking_for,
    deployments::{
        DeploymentId,
        ports::{DeploymentRepository, DeploymentService},
        service::DeploymentServiceImpl,
    },
    organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;
use autharie_postgres::cells::PostgresTenantAdministratorRepository;
use chrono::Utc;
use tracing::{error, info};

use crate::{
    AutharieService,
    infrastructure::{
        credentials::EnvelopeAdministratorVault, provisioner::LocalClusterProvisioner,
        role::permissions_in,
    },
    policy::{AuthariePolicy, PolicyContext},
};

impl AutharieService {
    #[transactional(data_plane, deployment)]
    pub async fn keep_tenant_administrator(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        administrator: InitialAdministrator,
    ) -> Result<bool, CoreError> {
        let speaking = speaking_for(&data_plane_repository, &identity).await?;
        let Some(deployment) = deployment_repository.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;
        if !deployment.is_pooled() {
            return Ok(false);
        }

        let Some(keys) = self.credential_keys() else {
            error!(
                %deployment_id,
                "no key manager is configured: the realm administrator credential was discarded"
            );
            return Ok(false);
        };

        let kept =
            EnvelopeAdministratorVault::new(PostgresTenantAdministratorRepository::new(&tx), keys)?
                .keep(deployment_id, administrator, Utc::now())
                .await?;
        if kept {
            info!(%deployment_id, "the realm administrator credential is kept until revealed");
        }
        Ok(kept)
    }

    #[transactional(deployment, user, data_plane, organisation)]
    pub async fn reveal_tenant_administrator(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<InitialAdministrator, CoreError> {
        let deployment = DeploymentServiceImpl::new(
            deployment_repository,
            user_repository,
            data_plane_repository,
            organisation_repository,
            LocalClusterProvisioner,
            self.placement_windows(),
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .get_deployment_for_organisation(identity.clone(), organisation_id, deployment_id)
        .await?;

        let permissions = permissions_in(&tx)
            .permissions_for_organisation(identity, organisation_id)
            .await?;
        if !PolicyContext::new(permissions).can(Permissions::ADMINISTRATOR) {
            return Err(AdministratorCredentialError::NotAnAdministrator.into());
        }

        let Some(keys) = self.credential_keys().filter(|_| deployment.is_pooled()) else {
            return Err(AdministratorCredentialError::NotIssued.into());
        };

        let administrator =
            EnvelopeAdministratorVault::new(PostgresTenantAdministratorRepository::new(&tx), keys)?
                .reveal(deployment.id, Utc::now())
                .await?;
        info!(%deployment_id, "the realm administrator credential was revealed");
        Ok(administrator)
    }

    #[transactional()]
    pub async fn purge_expired_tenant_administrators(&self) -> Result<u64, CoreError> {
        PostgresTenantAdministratorRepository::new(&tx)
            .purge_expired(Utc::now())
            .await
    }
}
