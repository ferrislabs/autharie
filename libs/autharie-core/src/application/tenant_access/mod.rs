use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    cells::{
        AdministratorCredentialError, AdministratorVault, CellRepository, InitialAdministrator,
        RealmRequests, TenantAdministrator, TenantAdministratorRepository,
    },
    dataplane::{herald_identity::speaking_for, value_objects::DataPlaneId},
    deployments::{
        Deployment, DeploymentId, DeploymentStatus,
        ports::{DeploymentRepository, DeploymentService},
        service::DeploymentServiceImpl,
    },
    organisation::OrganisationId,
    role::ports::PermissionProvider,
    user::ports::UserRepository,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;
use autharie_persistence::SharedTx;
use autharie_postgres::{
    action::PostgresActionRepository,
    cells::{PostgresCellRepository, PostgresTenantAdministratorRepository},
    deployments::PostgresDeploymentRepository,
    user::PostgresUserRepository,
};
use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use crate::{
    AutharieService,
    infrastructure::{
        credentials::EnvelopeAdministratorVault, provisioner::LocalClusterProvisioner,
        role::permissions_in,
    },
    policy::{AuthariePolicy, PolicyContext},
};

async fn require_administrator(
    tx: &SharedTx<'_>,
    identity: Identity,
    organisation_id: OrganisationId,
) -> Result<(), CoreError> {
    let permissions = permissions_in(tx)
        .permissions_for_organisation(identity, organisation_id)
        .await?;
    if PolicyContext::new(permissions).can(Permissions::ADMINISTRATOR) {
        Ok(())
    } else {
        Err(AdministratorCredentialError::NotAnAdministrator.into())
    }
}

fn serving_realm(deployment: &Deployment) -> Result<(), CoreError> {
    let serving = deployment.is_pooled()
        && deployment.deleted_at.is_none()
        && deployment.status == DeploymentStatus::Successful;
    if serving {
        Ok(())
    } else {
        Err(AdministratorCredentialError::NotARealm.into())
    }
}

pub struct ReportedIssue {
    pub dataplane: DataPlaneId,
    pub deployment: DeploymentId,
    pub request: Uuid,
}

impl AutharieService {
    #[transactional(data_plane, deployment)]
    pub async fn keep_tenant_administrator(
        &self,
        identity: Identity,
        reported: ReportedIssue,
        administrator: InitialAdministrator,
    ) -> Result<bool, CoreError> {
        let deployment_id = reported.deployment;
        let speaking = speaking_for(&data_plane_repository, &identity).await?;
        let Some(deployment) = deployment_repository.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;
        speaking.is(reported.dataplane)?;
        if serving_realm(&deployment).is_err() {
            info!(%deployment_id, "an administrator for a realm that no longer serves was ignored");
            return Ok(false);
        }

        let keys = self.credential_keys().ok_or_else(|| {
            CoreError::InternalError(
                "no key manager is configured to keep a realm administrator".to_string(),
            )
        })?;

        let kept =
            EnvelopeAdministratorVault::new(PostgresTenantAdministratorRepository::new(&tx), keys)?
                .keep(deployment_id, reported.request, administrator, Utc::now())
                .await?;
        if kept {
            info!(%deployment_id, "a realm administrator credential is kept until revealed");
        } else {
            info!(%deployment_id, "an administrator for an older or unknown request was ignored");
        }
        Ok(kept)
    }

    #[transactional(deployment, user, data_plane, organisation)]
    pub async fn issue_tenant_administrator(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<TenantAdministrator, CoreError> {
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
        require_administrator(&tx, identity.clone(), organisation_id).await?;
        serving_realm(&deployment)?;
        let cells = PostgresCellRepository::new(&tx);
        if cells.realm_refusal(deployment.id).await?.is_some() {
            return Err(AdministratorCredentialError::NotARealm.into());
        }
        let (Some(zone), Some(_)) = (self.deployment_domain(), self.credential_keys()) else {
            return Err(AdministratorCredentialError::NotARealm.into());
        };

        let administrators = PostgresTenantAdministratorRepository::new(&tx);
        administrators.lock(deployment.id).await?;
        let now = Utc::now();
        if let Some(record) = administrators.issue_record(deployment.id).await?
            && !record.may_issue_again(now)
        {
            return Err(AdministratorCredentialError::AlreadyIssuing.into());
        }

        let asker = PostgresUserRepository::new(&tx)
            .find_by_sub(identity.id())
            .await?
            .ok_or(CoreError::InvalidIdentity)?;
        let request = Uuid::new_v4();
        RealmRequests::new(
            cells,
            PostgresDeploymentRepository::new(&tx),
            PostgresActionRepository::new(&tx),
        )
        .issue_administrator(&deployment, zone, asker.id, request)
        .await?;
        administrators
            .request_issue(deployment.id, request, now)
            .await?;
        info!(%deployment_id, "a realm administrator credential was asked for");

        administrators
            .describe(deployment.id, now)
            .await?
            .ok_or_else(|| CoreError::InternalError("the issue was not recorded".to_string()))
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
        require_administrator(&tx, identity, organisation_id).await?;

        let Some(keys) = self.credential_keys().filter(|_| deployment.is_pooled()) else {
            return Err(AdministratorCredentialError::NotIssued.into());
        };

        let administrator =
            EnvelopeAdministratorVault::new(PostgresTenantAdministratorRepository::new(&tx), keys)?
                .reveal(deployment.id, Utc::now())
                .await?;
        info!(%deployment_id, "a realm administrator credential was revealed");
        Ok(administrator)
    }

    #[transactional()]
    pub async fn purge_expired_tenant_administrators(&self) -> Result<u64, CoreError> {
        PostgresTenantAdministratorRepository::new(&tx)
            .purge_expired(Utc::now())
            .await
    }
}
