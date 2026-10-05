use autharie_domain::{
    CoreError,
    dataplane::{
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{
            DataPlaneId, DataPlaneMode, DeploymentResources, PlacementRequest, Region,
        },
    },
    organisation::OrganisationId,
    version::Version,
};
use autharie_postgres::dataplane::PostgresDataPlaneRepository;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PooledDataPlanes {
    pool: PgPool,
}

impl PooledDataPlanes {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DataPlaneRepository for PooledDataPlanes {
    async fn find_by_herald_subject(&self, subject: &str) -> Result<Option<DataPlane>, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            find_by_herald_subject(subject)
        )
    }

    async fn find_by_id(&self, id: &DataPlaneId) -> Result<Option<DataPlane>, CoreError> {
        in_tx!(&self.pool, PostgresDataPlaneRepository, find_by_id(id))
    }

    async fn find_active_shared_by_region(
        &self,
        region: &Region,
    ) -> Result<Vec<DataPlane>, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            find_active_shared_by_region(region)
        )
    }

    async fn find_available(
        &self,
        request: PlacementRequest,
    ) -> Result<Option<DataPlane>, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            find_available(request)
        )
    }

    async fn region_is_served(&self, region: &Region) -> Result<bool, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            region_is_served(region)
        )
    }

    async fn region_blocked_by_deployment_count(
        &self,
        region: &Region,
        mode: DataPlaneMode,
        resources: DeploymentResources,
    ) -> Result<bool, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            region_blocked_by_deployment_count(region, mode, resources)
        )
    }

    async fn find_dedicated_for_organisation(
        &self,
        organisation_id: &OrganisationId,
        region: &Region,
    ) -> Result<Option<DataPlane>, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            find_dedicated_for_organisation(organisation_id, region)
        )
    }

    async fn list_all(&self) -> Result<Vec<DataPlane>, CoreError> {
        in_tx!(&self.pool, PostgresDataPlaneRepository, list_all())
    }

    async fn current_load(&self, id: &DataPlaneId) -> Result<u32, CoreError> {
        in_tx!(&self.pool, PostgresDataPlaneRepository, current_load(id))
    }

    async fn save(&self, dataplane: &DataPlane) -> Result<(), CoreError> {
        in_tx!(&self.pool, PostgresDataPlaneRepository, save(dataplane))
    }

    async fn touch_last_seen(
        &self,
        id: &DataPlaneId,
        at: DateTime<Utc>,
        operator_version: Option<Version>,
        gateway_address: Option<String>,
    ) -> Result<bool, CoreError> {
        in_tx!(
            &self.pool,
            PostgresDataPlaneRepository,
            touch_last_seen(id, at, operator_version, gateway_address)
        )
    }
}
