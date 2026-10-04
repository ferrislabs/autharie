use autharie_domain::{
    CoreError,
    dataplane::{entities::DataPlane, value_objects::DataPlaneId},
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = ClusterClaims, backend = Postgres)]
pub struct PostgresClusterClaims<'tx> {
    tx: SharedTx<'tx>,
}

fn database_error(action: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {action}: {error}"),
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresClusterClaims<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }

    pub async fn claim_provisioning(
        &self,
        lease_seconds: f64,
        limit: i64,
    ) -> Result<Vec<DataPlaneId>, CoreError> {
        let mut tx = self.tx.lock().await;
        let claimed = sqlx::query_scalar!(
            r#"
            UPDATE data_planes
            SET provisioning_claimed_until = now() + make_interval(secs => $1),
                updated_at = now()
            WHERE id IN (
                SELECT dp.id
                FROM data_planes dp
                JOIN deployments d ON d.id = dp.deployment_id
                WHERE dp.mode = 'customer'
                  AND dp.status = 'provisioning'
                  AND dp.herald_client_id IS NULL
                  AND d.deleted_at IS NULL
                  AND (dp.provisioning_claimed_until IS NULL
                       OR dp.provisioning_claimed_until < now())
                ORDER BY dp.created_at
                LIMIT $2
                FOR UPDATE OF dp SKIP LOCKED
            )
            RETURNING id
            "#,
            lease_seconds,
            limit
        )
        .fetch_all(&mut ***tx)
        .await
        .map_err(database_error("claim customer data planes to provision"))?;

        Ok(claimed.into_iter().map(DataPlaneId).collect())
    }

    pub async fn release_claim(&self, id: &DataPlaneId) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            UPDATE data_planes
            SET provisioning_claimed_until = NULL,
                updated_at = now()
            WHERE id = $1
            "#,
            id.0
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("release a provisioning claim"))?;

        Ok(())
    }

    pub async fn complete_provisioning(&self, plane: &DataPlane) -> Result<bool, CoreError> {
        let Some(herald) = plane.herald.as_ref() else {
            return Err(CoreError::InternalError(format!(
                "data plane {} was completed without a herald binding",
                plane.id
            )));
        };

        let mut tx = self.tx.lock().await;
        let affected = sqlx::query!(
            r#"
            UPDATE data_planes
            SET herald_client_id = $2,
                herald_subject = $3,
                capacity_cpu_millis = $4,
                capacity_memory_mib = $5,
                capacity_storage_gib = $6,
                capacity_max_deployments = $7,
                provisioning_claimed_until = NULL,
                updated_at = now()
            WHERE id = $1
              AND status = 'provisioning'
            "#,
            plane.id.0,
            herald.client_id,
            herald.subject,
            plane.capacity.cpu_millis() as i32,
            plane.capacity.memory_mib() as i32,
            plane.capacity.storage_gib() as i32,
            plane.capacity.max_deployments().map(|n| n as i32),
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("record a provisioned cluster"))?
        .rows_affected();

        Ok(affected > 0)
    }

    pub async fn fail_provisioning(
        &self,
        id: &DataPlaneId,
        reason: &str,
    ) -> Result<bool, CoreError> {
        let mut tx = self.tx.lock().await;
        let affected = sqlx::query!(
            r#"
            UPDATE data_planes
            SET status = 'failed',
                failure_reason = $2,
                provisioning_claimed_until = NULL,
                updated_at = now()
            WHERE id = $1
              AND status = 'provisioning'
            "#,
            id.0,
            reason
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("mark a provisioning data plane failed"))?
        .rows_affected();

        Ok(affected > 0)
    }

    pub async fn teardown_candidates(&self, limit: i64) -> Result<Vec<DataPlaneId>, CoreError> {
        let mut tx = self.tx.lock().await;
        let ids = sqlx::query_scalar!(
            r#"
            SELECT dp.id AS "id!"
            FROM data_planes dp
            LEFT JOIN deployments d ON d.id = dp.deployment_id
            WHERE dp.mode = 'customer'
              AND (dp.provisioning_claimed_until IS NULL
                   OR dp.provisioning_claimed_until < now())
              AND (
                    ((d.id IS NULL OR d.deleted_at IS NOT NULL)
                     AND dp.status NOT IN ('disabled', 'failed'))
                 OR ((d.id IS NULL OR d.deleted_at IS NOT NULL OR dp.status = 'failed')
                     AND EXISTS (
                         SELECT 1
                         FROM cluster_inventory ci
                         WHERE ci.data_plane_id = dp.id
                           AND ci.released_at IS NULL
                     ))
              )
            ORDER BY dp.created_at
            LIMIT $1
            "#,
            limit
        )
        .fetch_all(&mut ***tx)
        .await
        .map_err(database_error("list customer data planes to tear down"))?;

        Ok(ids.into_iter().map(DataPlaneId).collect())
    }

    pub async fn disable(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        let mut tx = self.tx.lock().await;
        let affected = sqlx::query!(
            r#"
            UPDATE data_planes
            SET status = 'disabled',
                updated_at = now()
            WHERE id = $1
              AND status NOT IN ('failed', 'disabled')
            "#,
            id.0
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("disable a torn down data plane"))?
        .rows_affected();

        Ok(affected > 0)
    }

    pub async fn awaiting_deletion(&self, limit: i64) -> Result<Vec<DataPlaneId>, CoreError> {
        let mut tx = self.tx.lock().await;
        let ids = sqlx::query_scalar!(
            r#"
            SELECT dp.id AS "id!"
            FROM data_planes dp
            JOIN deployments d ON d.id = dp.deployment_id
            WHERE dp.mode = 'customer'
              AND dp.status IN ('disabled', 'failed')
              AND d.status = 'deleting'
              AND d.deleted_at IS NOT NULL
              AND (dp.provisioning_claimed_until IS NULL
                   OR dp.provisioning_claimed_until < now())
              AND NOT EXISTS (
                  SELECT 1
                  FROM cluster_inventory ci
                  WHERE ci.data_plane_id = dp.id
                    AND ci.released_at IS NULL
              )
            ORDER BY dp.created_at
            LIMIT $1
            "#,
            limit
        )
        .fetch_all(&mut ***tx)
        .await
        .map_err(database_error(
            "list released customer data planes whose deployment is still deleting",
        ))?;

        Ok(ids.into_iter().map(DataPlaneId).collect())
    }
}
