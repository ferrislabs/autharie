use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    catalog::ports::ReleaseRepository,
    cells::{
        Cell, CellError, CellId, CellRepository, CellServiceImpl, CellStatus, CreateCellCommand,
        release_for_cells,
    },
    deployments::{
        DeploymentId, DeploymentKind, ports::DeploymentRepository, service::DeploymentServiceImpl,
    },
    user::ports::UserRepository,
};
use autharie_macros::transactional;
use autharie_persistence::SharedTx;
use autharie_postgres::{
    cells::PostgresCellRepository, deployments::PostgresDeploymentRepository,
    platform::PostgresOperatorRepository,
};

use crate::{
    AutharieService,
    application::deployment::Serving,
    infrastructure::{provisioner::LocalClusterProvisioner, role::permissions_in},
    policy::{AuthariePolicy, PlatformRightsPolicy},
};

impl AutharieService {
    #[transactional(
        cell,
        platform_operator,
        release,
        deployment,
        user,
        data_plane,
        organisation,
        backup_schedule,
        action
    )]
    pub async fn create_cell(
        &self,
        identity: Identity,
        command: CreateCellCommand,
    ) -> Result<Cell, CoreError> {
        let cells = CellServiceImpl::new(
            cell_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
        );
        cells.authorise_creation(identity.clone()).await?;

        let organisation = self.cells_organisation().ok_or(CellError::NotConfigured)?;
        let created_by = user_repository
            .find_by_sub(identity.id())
            .await?
            .ok_or(CoreError::InvalidIdentity)?
            .id;
        let releases = release_repository
            .list_for_kind(&DeploymentKind::Ferriskey)
            .await?;
        let version = release_for_cells(&releases)?;

        let instance = DeploymentServiceImpl::new(
            deployment_repository,
            user_repository,
            data_plane_repository,
            organisation_repository,
            LocalClusterProvisioner,
            self.placement_windows(),
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .place_as(
            created_by,
            command.instance(organisation, version, created_by)?,
        )
        .await?;
        let cell = cells.record(&command, &instance).await?;
        self.announce_new_deployment(
            &instance,
            Serving::PooledCell,
            backup_schedule_repository,
            action_repository,
        )
        .await?;

        Ok(cell)
    }

    #[transactional(cell, platform_operator)]
    pub async fn list_cells(&self, identity: Identity) -> Result<Vec<Cell>, CoreError> {
        CellServiceImpl::new(
            cell_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
        )
        .list(identity)
        .await
    }

    #[transactional(cell, platform_operator)]
    pub async fn get_cell(&self, identity: Identity, id: CellId) -> Result<Cell, CoreError> {
        CellServiceImpl::new(
            cell_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
        )
        .get(identity, id)
        .await
    }

    #[transactional(cell, platform_operator)]
    pub async fn drain_cell(&self, identity: Identity, id: CellId) -> Result<Cell, CoreError> {
        CellServiceImpl::new(
            cell_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
        )
        .drain(identity, id)
        .await
    }

    #[transactional(cell, platform_operator)]
    pub async fn retire_cell(&self, identity: Identity, id: CellId) -> Result<Cell, CoreError> {
        CellServiceImpl::new(
            cell_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
        )
        .retire(identity, id)
        .await
    }
}

pub(crate) async fn refuse_if_a_cell_runs_on(
    tx: &SharedTx<'_>,
    deployment: DeploymentId,
) -> Result<(), CoreError> {
    let cell = PostgresCellRepository::new(tx)
        .find_by_instance(deployment)
        .await?;

    match cell {
        Some(cell) if cell.status() != CellStatus::Retired => {
            Err(CellError::InstanceInUse { cell: cell.id().0 }.into())
        }
        _ => Ok(()),
    }
}

pub(crate) async fn settle_cell_instance(
    tx: &SharedTx<'_>,
    instance: DeploymentId,
) -> Result<Option<Cell>, CoreError> {
    let Some(deployment) = PostgresDeploymentRepository::new(tx)
        .get_by_id(instance)
        .await?
    else {
        return Ok(None);
    };

    CellServiceImpl::new(
        PostgresCellRepository::new(tx),
        PlatformRightsPolicy::new(PostgresOperatorRepository::new(tx)),
    )
    .settle_instance(&deployment)
    .await
}
