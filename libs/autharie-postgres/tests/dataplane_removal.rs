//! Removing a data plane (#removal), checked against a real Postgres.
//!
//! The rule lives in SQL: the row goes, the deployments already deleted go
//! with it through the foreign key, and a plane with infrastructure still to
//! release stays. See `heartbeat_activates.rs` for why these skip without
//! `DATABASE_URL`.

use autharie_domain::{
    CoreError,
    dataplane::{
        ports::{DataPlaneRepository, Removal},
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneStatus, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    user::UserId,
};
use autharie_persistence::in_scratch_tx;
use autharie_postgres::{
    dataplane::PostgresDataPlaneRepository, deployments::PostgresDeploymentRepository,
};
use chrono::Utc;
use uuid::Uuid;

mod support;
use support::pool;

const TEST_REGION: &str = "dataplane-removal-test";

async fn seed_organisation(
    tx: &autharie_persistence::SharedTx<'_>,
    label: &str,
) -> Result<(UserId, OrganisationId), CoreError> {
    let user_id = UserId(Uuid::new_v4());
    let organisation_id = OrganisationId(Uuid::new_v4());

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'bound', $3)")
        .bind(user_id.0)
        .bind(format!("{}@bound.test", user_id.0))
        .bind(user_id.0.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(|e| CoreError::DatabaseError {
            message: e.to_string(),
        })?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'active', 'free', 100, 100, 100, now(), now())",
    )
    .bind(organisation_id.0)
    .bind(label)
    .bind(format!("{label}-{}", organisation_id.0))
    .bind(user_id.0)
    .execute(&mut ***guard)
    .await
    .map_err(|e| CoreError::DatabaseError {
        message: e.to_string(),
    })?;

    Ok((user_id, organisation_id))
}

async fn seed_dataplane(tx: &autharie_persistence::SharedTx<'_>) -> Result<Uuid, CoreError> {
    let dataplanes = PostgresDataPlaneRepository::new(tx);

    let mut dataplane = autharie_domain::dataplane::entities::DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(TEST_REGION),
        Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
    );
    dataplane.status = DataPlaneStatus::Disabled;
    dataplanes.save(&dataplane).await?;

    Ok(dataplane.id.0)
}

fn deployment(
    organisation_id: OrganisationId,
    dataplane_id: Uuid,
    created_by: UserId,
    name: &str,
) -> Deployment {
    let at = Utc::now();

    Deployment {
        id: DeploymentId(Uuid::new_v4()),
        organisation_id,
        dataplane_id: autharie_domain::dataplane::value_objects::DataPlaneId(dataplane_id),
        name: DeploymentName(name.to_string()),
        kind: DeploymentKind::Ferriskey,
        version: autharie_domain::version::Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: format!("plane-removal-{name}"),
        environment: autharie_domain::deployments::environment::Environment::Development,
        offer: None,
        restored_from: None,
        resources: autharie_domain::dataplane::value_objects::DeploymentResources::DEFAULT,
        created_by,
        created_at: at,
        updated_at: at,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: Default::default(),
        maintenance_window: None,
        network_access: autharie_domain::deployments::network::NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    }
}

fn db_error(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

#[tokio::test]
async fn a_plane_with_nothing_to_release_is_removed_with_its_deleted_deployments() {
    let Some(pool) = pool().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let result: Result<(Removal, bool, i64), CoreError> =
        in_scratch_tx(&pool, db_error, async |tx| {
            let dataplane_id = seed_dataplane(&tx).await?;
            let (user_id, organisation_id) = seed_organisation(&tx, "acme").await?;
            let mut gone = deployment(organisation_id, dataplane_id, user_id, "gone");
            gone.status = DeploymentStatus::Deleted;
            gone.deleted_at = Some(Utc::now());
            let gone_id = gone.id.0;
            PostgresDeploymentRepository::new(&tx).insert(gone).await?;

            let planes = PostgresDataPlaneRepository::new(&tx);
            let id = DataPlaneId(dataplane_id);
            let removal = planes.remove(&id).await?;
            let still_there = planes.find_by_id(&id).await?.is_some();

            let mut guard = tx.lock().await;
            let orphans: i64 = sqlx::query_scalar("SELECT count(*) FROM deployments WHERE id = $1")
                .bind(gone_id)
                .fetch_one(&mut ***guard)
                .await
                .map_err(db_error)?;

            Ok((removal, still_there, orphans))
        })
        .await;

    let (removal, still_there, orphans) = result.expect("the transaction committed");
    assert_eq!(removal, Removal::Removed);
    assert!(!still_there);
    assert_eq!(orphans, 0, "the deleted deployment went with its plane");
}

#[tokio::test]
async fn a_plane_with_infrastructure_still_to_release_stays() {
    let Some(pool) = pool().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let result: Result<(Removal, bool), CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let dataplane_id = seed_dataplane(&tx).await?;
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO cluster_inventory (data_plane_id, kind, provider_id) \
                     VALUES ($1, 'cluster', 'scw-cluster-1')",
            )
            .bind(dataplane_id)
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        }

        let planes = PostgresDataPlaneRepository::new(&tx);
        let id = DataPlaneId(dataplane_id);
        let removal = planes.remove(&id).await?;
        let still_there = planes.find_by_id(&id).await?.is_some();

        Ok((removal, still_there))
    })
    .await;

    let (removal, still_there) = result.expect("the transaction committed");
    assert_eq!(removal, Removal::InfrastructureRemains);
    assert!(still_there, "the row that remembers the cluster must stay");
}

#[tokio::test]
async fn released_infrastructure_does_not_hold_a_plane_back() {
    let Some(pool) = pool().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let result: Result<Removal, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let dataplane_id = seed_dataplane(&tx).await?;
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO cluster_inventory \
                 (data_plane_id, kind, provider_id, released_at) \
                 VALUES ($1, 'cluster', 'scw-cluster-1', now())",
            )
            .bind(dataplane_id)
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        }

        PostgresDataPlaneRepository::new(&tx)
            .remove(&DataPlaneId(dataplane_id))
            .await
    })
    .await;

    assert_eq!(result.expect("the transaction committed"), Removal::Removed);
}

#[tokio::test]
async fn a_plane_that_hosts_a_cell_stays() {
    let Some(pool) = pool().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let result: Result<(Removal, bool), CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let dataplane_id = seed_dataplane(&tx).await?;
        let (user_id, organisation_id) = seed_organisation(&tx, "acme").await?;
        let instance = deployment(organisation_id, dataplane_id, user_id, "cell-instance");
        let instance_id = instance.id.0;
        PostgresDeploymentRepository::new(&tx)
            .insert(instance)
            .await?;
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO cells \
                 (id, region, data_plane_id, instance_deployment_id, status, capacity, created_at) \
                 VALUES ($1, $2, $3, $4, 'retired', 200, now())",
            )
            .bind(Uuid::new_v4())
            .bind(TEST_REGION)
            .bind(dataplane_id)
            .bind(instance_id)
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        }

        let planes = PostgresDataPlaneRepository::new(&tx);
        let id = DataPlaneId(dataplane_id);
        let removal = planes.remove(&id).await?;
        let still_there = planes.find_by_id(&id).await?.is_some();

        Ok((removal, still_there))
    })
    .await;

    let (removal, still_there) = result.expect("the transaction committed");
    assert_eq!(removal, Removal::HostsCells);
    assert!(still_there, "a plane that hosts a cell cannot be forgotten");
}
