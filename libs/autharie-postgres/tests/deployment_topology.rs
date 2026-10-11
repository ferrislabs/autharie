use autharie_domain::{
    CoreError,
    dataplane::{
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{
            Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneStatus, DeploymentResources,
            Region,
        },
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        environment::Environment, network::NetworkAccess, ports::DeploymentRepository,
        topology::Topology,
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

fn db_error(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, db_error, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

struct World {
    user: UserId,
    organisation: OrganisationId,
    dataplane: DataPlaneId,
}

async fn seed_world(tx: &SharedTx<'_>) -> Result<World, CoreError> {
    let user = UserId(Uuid::new_v4());
    let organisation = OrganisationId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'topology', $3)")
            .bind(user.0)
            .bind(format!("{}@topology.test", user.0))
            .bind(user.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'topology', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
        )
        .bind(organisation.0)
        .bind(format!("topology-{}", organisation.0))
        .bind(user.0)
        .execute(&mut ***guard)
        .await
        .map_err(db_error)?;
    }

    let mut plane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(format!("topology-plane-{}", Uuid::new_v4())),
        Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
    );
    plane.status = DataPlaneStatus::Active;
    PostgresDataPlaneRepository::new(tx).save(&plane).await?;

    Ok(World {
        user,
        organisation,
        dataplane: plane.id,
    })
}

fn deployment(world: &World, topology: Topology) -> Deployment {
    let at = Utc::now();
    let name = format!("t{}", &Uuid::new_v4().simple().to_string()[..12]);

    Deployment {
        topology,
        id: DeploymentId(Uuid::new_v4()),
        organisation_id: world.organisation,
        dataplane_id: world.dataplane,
        namespace: format!("topology-{name}"),
        name: DeploymentName(name),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        environment: Environment::Development,
        offer: None,
        restored_from: None,
        resources: DeploymentResources::DEFAULT,
        created_by: world.user,
        created_at: at,
        updated_at: at,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: Default::default(),
        maintenance_window: None,
        network_access: NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    }
}

async fn violated_constraint(
    tx: &SharedTx<'_>,
    world: &World,
    replicas: i16,
    database_instances: i16,
) -> Option<String> {
    let name = format!("c{}", &Uuid::new_v4().simple().to_string()[..12]);
    let mut guard = tx.lock().await;
    let error = sqlx::query(
        "INSERT INTO deployments \
         (id, organisation_id, dataplane_id, name, kind, status, namespace, environment, \
          version, cpu_millis, memory_mib, storage_gib, created_by, created_at, updated_at, \
          auto_upgrade, hostname_slug, distribution, replicas, database_instances) \
         VALUES ($1, $2, $3, $4, 'ferriskey', 'successful', $5, 'development', '26.0.1', \
                 500, 1024, 1, $6, now(), now(), 'manual', $4, 'shared', $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(world.organisation.0)
    .bind(world.dataplane.0)
    .bind(&name)
    .bind(format!("topology-{name}"))
    .bind(world.user.0)
    .bind(replicas)
    .bind(database_instances)
    .execute(&mut ***guard)
    .await
    .expect_err("the row is outside the range");

    error
        .as_database_error()
        .and_then(|database| database.constraint())
        .map(str::to_string)
}

#[tokio::test]
async fn a_topology_survives_a_round_trip() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let deployments = PostgresDeploymentRepository::new(&tx);
        let stored = deployment(&world, Topology::new(3, 2).expect("in range"));

        deployments.insert(stored.clone()).await?;

        let read = deployments.get_by_id(stored.id).await?.expect("stored");
        assert_eq!(read.topology, Topology::new(3, 2).expect("in range"));
        assert_eq!(read, stored);

        let listed = deployments.list_by_organisation(world.organisation).await?;
        assert_eq!(listed[0].topology, stored.topology);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_row_written_without_a_topology_reads_as_single() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let name = format!("o{}", &Uuid::new_v4().simple().to_string()[..12]);
        let id = Uuid::new_v4();
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO deployments \
                 (id, organisation_id, dataplane_id, name, kind, status, namespace, \
                  environment, version, cpu_millis, memory_mib, storage_gib, created_by, \
                  created_at, updated_at, auto_upgrade, hostname_slug, distribution) \
                 VALUES ($1, $2, $3, $4, 'ferriskey', 'successful', $5, 'development', \
                         '26.0.1', 500, 1024, 1, $6, now(), now(), 'manual', $4, 'shared')",
            )
            .bind(id)
            .bind(world.organisation.0)
            .bind(world.dataplane.0)
            .bind(&name)
            .bind(format!("topology-{name}"))
            .bind(world.user.0)
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        }

        let read = PostgresDeploymentRepository::new(&tx)
            .get_by_id(DeploymentId(id))
            .await?
            .expect("stored");
        assert_eq!(read.topology, Topology::SINGLE);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn update_keeps_and_changes_the_topology() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let deployments = PostgresDeploymentRepository::new(&tx);
        let mut stored = deployment(&world, Topology::HIGH_AVAILABILITY);
        deployments.insert(stored.clone()).await?;

        stored.status = DeploymentStatus::Maintenance;
        deployments.update(stored.clone()).await?;
        let read = deployments.get_by_id(stored.id).await?.expect("stored");
        assert_eq!(read.topology, Topology::HIGH_AVAILABILITY);

        stored.topology = Topology::new(4, 3).expect("in range");
        deployments.update(stored.clone()).await?;
        let read = deployments.get_by_id(stored.id).await?.expect("stored");
        assert_eq!(read.topology, Topology::new(4, 3).expect("in range"));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn the_database_refuses_replicas_out_of_range() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let constraint = violated_constraint(&tx, &world, 6, 1).await;
        assert_eq!(constraint.as_deref(), Some("deployments_replicas_in_range"));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn the_database_refuses_zero_replicas() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let constraint = violated_constraint(&tx, &world, 0, 1).await;
        assert_eq!(constraint.as_deref(), Some("deployments_replicas_in_range"));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn the_database_refuses_database_instances_out_of_range() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let constraint = violated_constraint(&tx, &world, 1, 6).await;
        assert_eq!(
            constraint.as_deref(),
            Some("deployments_database_instances_in_range")
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn the_database_refuses_zero_database_instances() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let constraint = violated_constraint(&tx, &world, 1, 0).await;
        assert_eq!(
            constraint.as_deref(),
            Some("deployments_database_instances_in_range")
        );
        Ok(())
    })
    .await;
}
