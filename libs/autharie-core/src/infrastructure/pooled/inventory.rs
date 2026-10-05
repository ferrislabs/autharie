use autharie_domain::{
    CoreError,
    dataplane::{
        inventory::{ClusterInventory, ProvisionedResource},
        value_objects::DataPlaneId,
    },
};
use autharie_postgres::dataplane::PostgresClusterInventory;
use sqlx::PgPool;

#[derive(Clone)]
pub struct PooledInventory {
    pool: PgPool,
}

impl PooledInventory {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ClusterInventory for PooledInventory {
    async fn record(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        in_tx!(
            &self.pool,
            PostgresClusterInventory,
            record(data_plane_id, resource)
        )
    }

    async fn unreleased(
        &self,
        data_plane_id: &DataPlaneId,
    ) -> Result<Vec<ProvisionedResource>, CoreError> {
        in_tx!(
            &self.pool,
            PostgresClusterInventory,
            unreleased(data_plane_id)
        )
    }

    async fn mark_released(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        in_tx!(
            &self.pool,
            PostgresClusterInventory,
            mark_released(data_plane_id, resource)
        )
    }
}
