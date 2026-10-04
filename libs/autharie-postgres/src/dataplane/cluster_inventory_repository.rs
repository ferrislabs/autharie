use autharie_domain::{
    CoreError,
    dataplane::{
        inventory::{ClusterInventory, ProvisionedResource, ResourceKind},
        value_objects::DataPlaneId,
    },
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::Utc;

fn parse_kind(raw: &str) -> Result<ResourceKind, CoreError> {
    match raw {
        "cluster" => Ok(ResourceKind::Cluster),
        "node_pool" => Ok(ResourceKind::NodePool),
        "private_network" => Ok(ResourceKind::PrivateNetwork),
        other => Err(CoreError::InternalError(format!(
            "unknown provisioned resource kind '{other}'"
        ))),
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = ClusterInventory, backend = Postgres)]
pub struct PostgresClusterInventory<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresClusterInventory<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl ClusterInventory for PostgresClusterInventory<'_> {
    async fn record(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO cluster_inventory (data_plane_id, kind, provider_id, recorded_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (data_plane_id, kind, provider_id) DO NOTHING
            "#,
                data_plane_id.0,
                resource.kind.as_str(),
                resource.provider_id,
                Utc::now(),
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to record a provisioned resource: {e}"),
        })?;

        Ok(())
    }

    async fn unreleased(
        &self,
        data_plane_id: &DataPlaneId,
    ) -> Result<Vec<ProvisionedResource>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            SELECT kind, provider_id
            FROM cluster_inventory
            WHERE data_plane_id = $1
              AND released_at IS NULL
            ORDER BY recorded_at DESC, provider_id DESC
            "#,
                data_plane_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to list unreleased resources: {e}"),
        })?;

        rows.into_iter()
            .map(|row| {
                Ok(ProvisionedResource {
                    kind: parse_kind(&row.kind)?,
                    provider_id: row.provider_id,
                })
            })
            .collect()
    }

    async fn mark_released(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE cluster_inventory
            SET released_at = $4
            WHERE data_plane_id = $1
              AND kind = $2
              AND provider_id = $3
              AND released_at IS NULL
            "#,
                data_plane_id.0,
                resource.kind.as_str(),
                resource.provider_id,
                Utc::now(),
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to release a provisioned resource: {e}"),
        })?;

        Ok(())
    }
}
