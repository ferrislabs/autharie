use std::time::Duration;

use autharie_domain::{
    CoreError,
    dataplane::{
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{DataPlaneAllocation, DataPlaneId},
    },
    deployments::{distribution::Distribution, ports::DeploymentRepository},
};
use autharie_persistence::{SharedTx, with_tx};
use autharie_postgres::{
    dataplane::{PostgresClusterClaims, PostgresDataPlaneRepository},
    deployments::PostgresDeploymentRepository,
};
use chrono::Utc;
use sqlx::PgPool;
use tracing::error;

use crate::customer_clusters::{ClaimedCluster, CustomerClusterQueue};

const MISSING_DEPLOYMENT: &str = "the deployment this cluster was for no longer exists";
const NOT_CUSTOMER_CLOUD: &str =
    "the deployment this cluster was for is not a customer cloud deployment";

#[derive(Clone)]
pub struct PostgresClusterQueue {
    pool: PgPool,
}

impl PostgresClusterQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn resolve(
    tx: &SharedTx<'_>,
    data_plane: DataPlane,
) -> Result<Result<ClaimedCluster, &'static str>, CoreError> {
    let DataPlaneAllocation::Customer {
        deployment_id,
        credential_id,
        ..
    } = data_plane.allocation
    else {
        return Ok(Err(NOT_CUSTOMER_CLOUD));
    };

    let Some(deployment) = PostgresDeploymentRepository::new(tx)
        .get_by_id(deployment_id)
        .await?
    else {
        return Ok(Err(MISSING_DEPLOYMENT));
    };

    match deployment.distribution {
        Distribution::CustomerCloud { profile, .. } => Ok(Ok(ClaimedCluster {
            data_plane,
            deployment_id,
            credential_id,
            profile,
            resources: deployment.resources,
        })),
        _ => Ok(Err(NOT_CUSTOMER_CLOUD)),
    }
}

impl CustomerClusterQueue for PostgresClusterQueue {
    async fn claim(&self, lease: Duration, limit: u32) -> Result<Vec<ClaimedCluster>, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            let claims = PostgresClusterClaims::new(&tx);
            let ids = claims
                .claim_provisioning(lease.as_secs_f64(), i64::from(limit))
                .await?;

            let data_planes = PostgresDataPlaneRepository::new(&tx);
            let mut claimed = Vec::with_capacity(ids.len());
            for id in ids {
                let Some(data_plane) = data_planes.find_by_id(&id).await? else {
                    continue;
                };
                match resolve(&tx, data_plane).await? {
                    Ok(cluster) => claimed.push(cluster),
                    Err(reason) => {
                        error!(data_plane_id = %id, reason, "a claimed data plane cannot be provisioned");
                        claims.fail_provisioning(&id, reason).await?;
                    }
                }
            }
            Ok(claimed)
        })
        .await
    }

    async fn complete(&self, data_plane: &DataPlane) -> Result<bool, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            PostgresClusterClaims::new(&tx)
                .complete_provisioning(data_plane)
                .await
        })
        .await
    }

    async fn fail(&self, id: &DataPlaneId, reason: &str) -> Result<bool, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            PostgresClusterClaims::new(&tx)
                .fail_provisioning(id, reason)
                .await
        })
        .await
    }

    async fn teardown_candidates(&self, limit: u32) -> Result<Vec<DataPlane>, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            let ids = PostgresClusterClaims::new(&tx)
                .teardown_candidates(i64::from(limit))
                .await?;

            let data_planes = PostgresDataPlaneRepository::new(&tx);
            let mut found = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(data_plane) = data_planes.find_by_id(&id).await? {
                    found.push(data_plane);
                }
            }
            Ok(found)
        })
        .await
    }

    async fn disable(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            PostgresClusterClaims::new(&tx).disable(id).await
        })
        .await
    }

    async fn awaiting_deletion(&self, limit: u32) -> Result<Vec<DataPlaneId>, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            PostgresClusterClaims::new(&tx)
                .awaiting_deletion(i64::from(limit))
                .await
        })
        .await
    }

    async fn confirm_deleted(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        with_tx(&self.pool, autharie_postgres::map_sqlx_error, async |tx| {
            let Some(data_plane) = PostgresDataPlaneRepository::new(&tx).find_by_id(id).await?
            else {
                return Ok(false);
            };
            let DataPlaneAllocation::Customer { deployment_id, .. } = data_plane.allocation else {
                return Ok(false);
            };

            let deployments = PostgresDeploymentRepository::new(&tx);
            let Some(mut deployment) = deployments.get_by_id(deployment_id).await? else {
                return Ok(false);
            };
            if !deployment.confirm_deletion(Utc::now()) {
                return Ok(false);
            }
            deployments.update(deployment).await?;
            Ok(true)
        })
        .await
    }
}
