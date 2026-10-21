use autharie_domain::{
    CoreError,
    dataplane::{
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{
            Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneStatus, DeploymentResources,
            PlaneLoad, Region,
        },
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    dataplane::PostgresDataPlaneRepository, deployments::PostgresDeploymentRepository,
};
use chrono::Utc;
use uuid::Uuid;

mod support;
use support::pool;

const TEST_REGION: &str = "dataplane-load-test";

fn map_err(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

async fn seed_user_and_organisation(
    tx: &SharedTx<'_>,
) -> Result<(UserId, OrganisationId), CoreError> {
    let user_id = UserId(Uuid::new_v4());
    let organisation_id = OrganisationId(Uuid::new_v4());

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'load', $3)")
        .bind(user_id.0)
        .bind(format!("{}@load.test", user_id.0))
        .bind(user_id.0.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'load', $2, $3, 'active', 'free', 9, 1, 1, now(), now())",
    )
    .bind(organisation_id.0)
    .bind(organisation_id.0.to_string())
    .bind(user_id.0)
    .execute(&mut ***guard)
    .await
    .map_err(map_err)?;

    Ok((user_id, organisation_id))
}

fn capacity() -> Capacity {
    Capacity::new(4_000, 8_192, 100).expect("non-zero capacity")
}

async fn a_plane(
    planes: &PostgresDataPlaneRepository<'_>,
    capacity: Capacity,
) -> Result<DataPlane, CoreError> {
    let mut plane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(TEST_REGION),
        capacity,
    );
    plane.status = DataPlaneStatus::Active;
    planes.save(&plane).await?;

    Ok(plane)
}

fn deployment(
    plane: DataPlaneId,
    organisation: OrganisationId,
    created_by: UserId,
    status: DeploymentStatus,
    resources: DeploymentResources,
) -> Deployment {
    let at = Utc::now();
    let id = DeploymentId(Uuid::new_v4());

    Deployment {
        topology: Default::default(),
        id,
        organisation_id: organisation,
        dataplane_id: plane,
        name: DeploymentName(format!("load-{}", id.0.simple())),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(26, 0, 1),
        status,
        namespace: format!("load-{}", id.0.simple()),
        environment: autharie_domain::deployments::environment::Environment::Development,
        offer: None,
        restored_from: None,
        resources,
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

fn sized(cpu_millis: u32, memory_mib: u32, storage_gib: u32) -> DeploymentResources {
    DeploymentResources {
        cpu_millis,
        memory_mib,
        storage_gib,
    }
}

#[tokio::test]
async fn the_load_counts_every_live_deployment_but_the_one_left_out() {
    let Some(pool) = pool().await else {
        return;
    };

    let load: Result<(PlaneLoad, PlaneLoad), CoreError> =
        in_scratch_tx(&pool, map_err, async |tx| {
            let planes = PostgresDataPlaneRepository::new(&tx);
            let deployments = PostgresDeploymentRepository::new(&tx);
            let plane = a_plane(&planes, capacity()).await?;
            let elsewhere = a_plane(&planes, capacity()).await?;
            let (user, organisation) = seed_user_and_organisation(&tx).await?;

            let resized = deployment(
                plane.id,
                organisation,
                user,
                DeploymentStatus::Successful,
                sized(1_000, 2_048, 10),
            );
            let failed = deployment(
                plane.id,
                organisation,
                user,
                DeploymentStatus::Failed,
                sized(500, 1_024, 1),
            );
            let deleted = deployment(
                plane.id,
                organisation,
                user,
                DeploymentStatus::Successful,
                sized(3_000, 6_144, 80),
            );
            let neighbour = deployment(
                elsewhere.id,
                organisation,
                user,
                DeploymentStatus::Successful,
                sized(2_000, 4_096, 40),
            );
            for inserted in [&resized, &failed, &deleted, &neighbour] {
                deployments.insert(inserted.clone()).await?;
            }
            deployments.delete(deleted.id).await?;

            let without_it = planes
                .load_excluding(&plane.id, &resized.id)
                .await?
                .expect("the plane exists");
            let with_every_one = planes
                .load_excluding(&plane.id, &DeploymentId(Uuid::new_v4()))
                .await?
                .expect("the plane exists");

            Ok((without_it, with_every_one))
        })
        .await;

    let (without_it, with_every_one) = load.expect("the transaction committed");

    assert_eq!(without_it.capacity, capacity());
    assert_eq!(
        without_it.used,
        sized(500, 1_024, 1),
        "a failed deployment still holds its room; a deleted one and another plane's do not"
    );
    assert_eq!(with_every_one.used, sized(1_500, 3_072, 11));
}

#[tokio::test]
async fn a_plane_holding_only_the_deployment_left_out_has_nothing_used() {
    let Some(pool) = pool().await else {
        return;
    };

    let load: Result<PlaneLoad, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        let planes = PostgresDataPlaneRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);
        let plane = a_plane(&planes, capacity()).await?;
        let (user, organisation) = seed_user_and_organisation(&tx).await?;
        let alone = deployment(
            plane.id,
            organisation,
            user,
            DeploymentStatus::Successful,
            sized(1_000, 2_048, 10),
        );
        deployments.insert(alone.clone()).await?;

        Ok(planes
            .load_excluding(&plane.id, &alone.id)
            .await?
            .expect("the plane exists"))
    })
    .await;

    assert_eq!(
        load.expect("the transaction committed").used,
        sized(0, 0, 0)
    );
}

#[tokio::test]
async fn the_capacity_keeps_its_count_bound() {
    let Some(pool) = pool().await else {
        return;
    };
    let bounded = capacity().with_max_deployments(3).expect("a bound");

    let load: Result<PlaneLoad, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        let planes = PostgresDataPlaneRepository::new(&tx);
        let plane = a_plane(&planes, bounded).await?;

        Ok(planes
            .load_excluding(&plane.id, &DeploymentId(Uuid::new_v4()))
            .await?
            .expect("the plane exists"))
    })
    .await;

    assert_eq!(load.expect("the transaction committed").capacity, bounded);
}

#[tokio::test]
async fn a_plane_that_does_not_exist_has_no_load() {
    let Some(pool) = pool().await else {
        return;
    };

    let load: Result<Option<PlaneLoad>, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        PostgresDataPlaneRepository::new(&tx)
            .load_excluding(&DataPlaneId(Uuid::new_v4()), &DeploymentId(Uuid::new_v4()))
            .await
    })
    .await;

    assert_eq!(load.expect("the transaction committed"), None);
}
