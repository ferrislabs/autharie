use autharie_domain::{
    CoreError,
    cells::{Cell, CellError, CellId, CellPolicy, CellRepository, CellStatus, RealmName},
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
        distribution::Distribution, environment::Environment, network::NetworkAccess,
        ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, in_scratch_tx, with_tx};
use autharie_postgres::{
    cells::PostgresCellRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository,
};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

mod support;
use support::pool;

fn db_error(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let Some(pool) = pool().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };

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
    region: Region,
    instance: DeploymentId,
}

async fn seed_world(tx: &SharedTx<'_>) -> Result<World, CoreError> {
    let user = UserId(Uuid::new_v4());
    let organisation = OrganisationId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'cells', $3)")
            .bind(user.0)
            .bind(format!("{}@cells.test", user.0))
            .bind(user.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'cells', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
        )
        .bind(organisation.0)
        .bind(format!("cells-{}", organisation.0))
        .bind(user.0)
        .execute(&mut ***guard)
        .await
        .map_err(db_error)?;
    }

    let mut plane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(format!("cells-plane-{}", Uuid::new_v4())),
        Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
    );
    plane.status = DataPlaneStatus::Active;
    PostgresDataPlaneRepository::new(tx).save(&plane).await?;

    let mut world = World {
        user,
        organisation,
        dataplane: plane.id,
        region: Region::new(format!("cells-{}", Uuid::new_v4())),
        instance: DeploymentId(Uuid::new_v4()),
    };
    let instance = deployment(&world, &unique("instance"), Distribution::Shared);
    world.instance = instance.id;
    PostgresDeploymentRepository::new(tx)
        .insert(instance)
        .await?;

    Ok(world)
}

fn unique(prefix: &str) -> String {
    format!("{prefix}{}", &Uuid::new_v4().simple().to_string()[..12])
}

fn deployment(world: &World, name: &str, distribution: Distribution) -> Deployment {
    let at = Utc::now();

    Deployment {
        id: DeploymentId(Uuid::new_v4()),
        organisation_id: world.organisation,
        dataplane_id: world.dataplane,
        name: DeploymentName(name.to_string()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: format!("cells-{name}"),
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
        distribution,
    }
}

fn pooled_in(world: &World, cell: CellId, realm: &str) -> Deployment {
    deployment(
        world,
        realm,
        Distribution::Pooled {
            cell_id: cell,
            realm: RealmName::try_from(realm).expect("a valid realm"),
        },
    )
}

fn provisioning(world: &World, region: &Region, capacity: u16, age: i64) -> Cell {
    Cell::new(
        CellId(Uuid::new_v4()),
        region.clone(),
        world.dataplane,
        world.instance,
        &CellPolicy::new(capacity, 0).expect("a valid policy"),
        DateTime::<Utc>::UNIX_EPOCH + Duration::days(age),
    )
}

async fn open_cell(
    cells: &PostgresCellRepository<'_>,
    world: &World,
    region: &Region,
    capacity: u16,
    age: i64,
) -> Result<CellId, CoreError> {
    let cell = provisioning(world, region, capacity, age);
    cells.create(&cell).await?;
    cells.set_status(cell.id(), CellStatus::Open).await?;
    Ok(cell.id())
}

async fn fill(
    deployments: &PostgresDeploymentRepository<'_>,
    world: &World,
    cell: CellId,
    count: usize,
) -> Result<Vec<DeploymentId>, CoreError> {
    let mut ids = Vec::new();
    for _ in 0..count {
        let tenant = pooled_in(world, cell, &unique("t"));
        ids.push(tenant.id);
        deployments.insert(tenant).await?;
    }
    Ok(ids)
}

async fn placed(cells: &PostgresCellRepository<'_>, region: &Region) -> Option<CellId> {
    cells
        .place(region)
        .await
        .expect("placement ran")
        .map(|cell| cell.id())
}

/// @spec-fpr-2
#[tokio::test]
async fn place_takes_the_fullest_open_cell_with_room_and_stays_in_its_region() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);
        let elsewhere = Region::new(format!("elsewhere-{}", Uuid::new_v4()));

        let empty = open_cell(&cells, &world, &world.region, 5, 0).await?;
        let fuller = open_cell(&cells, &world, &world.region, 5, 1).await?;
        let other_region = open_cell(&cells, &world, &elsewhere, 5, 2).await?;
        fill(&deployments, &world, fuller, 2).await?;
        fill(&deployments, &world, other_region, 4).await?;

        assert_eq!(placed(&cells, &world.region).await, Some(fuller));
        assert_ne!(placed(&cells, &world.region).await, Some(empty));
        assert_eq!(placed(&cells, &elsewhere).await, Some(other_region));
        Ok(())
    })
    .await;
}

/// @spec-fpr-2
#[tokio::test]
async fn place_breaks_a_tie_with_the_oldest_cell() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);

        let young = open_cell(&cells, &world, &world.region, 5, 9).await?;
        let old = open_cell(&cells, &world, &world.region, 5, 1).await?;

        assert_eq!(placed(&cells, &world.region).await, Some(old));
        assert_ne!(Some(young), Some(old));
        Ok(())
    })
    .await;
}

/// @spec-fpr-3
#[tokio::test]
async fn a_cell_at_capacity_reads_as_full_and_is_no_longer_placed() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);

        let first = open_cell(&cells, &world, &world.region, 3, 0).await?;
        let second = open_cell(&cells, &world, &world.region, 3, 1).await?;
        fill(&deployments, &world, first, 2).await?;

        assert_eq!(placed(&cells, &world.region).await, Some(first));
        fill(&deployments, &world, first, 1).await?;

        let full = cells.find(first).await?.expect("the cell exists");
        assert_eq!(full.status(), CellStatus::Full);
        assert_eq!(full.realms(), 3);
        assert_eq!(placed(&cells, &world.region).await, Some(second));
        Ok(())
    })
    .await;
}

/// @spec-fpr-3
#[tokio::test]
async fn place_is_empty_when_every_cell_is_full() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);

        let only = open_cell(&cells, &world, &world.region, 2, 0).await?;
        fill(&deployments, &world, only, 2).await?;

        assert_eq!(placed(&cells, &world.region).await, None);
        Ok(())
    })
    .await;
}

/// @spec-fpr-8
/// @spec-fpr-9
#[tokio::test]
async fn release_is_idempotent_and_reopens_a_full_cell() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);

        let cell = open_cell(&cells, &world, &world.region, 2, 0).await?;
        let tenants = fill(&deployments, &world, cell, 2).await?;
        assert_eq!(
            cells.find(cell).await?.expect("exists").status(),
            CellStatus::Full
        );
        assert_eq!(placed(&cells, &world.region).await, None);

        assert!(cells.release(tenants[0]).await?);
        assert!(!cells.release(tenants[0]).await?);

        let reopened = cells.find(cell).await?.expect("exists");
        assert_eq!(reopened.status(), CellStatus::Open);
        assert_eq!(reopened.realms(), 1);
        assert_eq!(placed(&cells, &world.region).await, Some(cell));
        Ok(())
    })
    .await;
}

/// @spec-fpr-8
#[tokio::test]
async fn releasing_an_unknown_deployment_changes_nothing() {
    scratch(async |tx| {
        let cells = PostgresCellRepository::new(&tx);

        assert!(!cells.release(DeploymentId(Uuid::new_v4())).await?);
        Ok(())
    })
    .await;
}

/// @spec-fpr-8
#[tokio::test]
async fn updating_a_released_deployment_does_not_take_its_slot_back() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);

        let cell = open_cell(&cells, &world, &world.region, 2, 0).await?;
        let tenants = fill(&deployments, &world, cell, 1).await?;
        assert!(cells.release(tenants[0]).await?);

        let mut tenant = deployments.get_by_id(tenants[0]).await?.expect("exists");
        tenant.status = DeploymentStatus::Deleted;
        deployments.update(tenant).await?;

        assert_eq!(cells.find(cell).await?.expect("exists").realms(), 0);
        Ok(())
    })
    .await;
}

/// @spec-fpr-10
#[tokio::test]
async fn a_cell_that_is_not_open_is_never_placed() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);

        let cell = provisioning(&world, &world.region, 5, 0);
        cells.create(&cell).await?;
        assert_eq!(placed(&cells, &world.region).await, None);

        for status in [
            CellStatus::Draining,
            CellStatus::Retired,
            CellStatus::Failed,
        ] {
            cells.set_status(cell.id(), status).await?;
            assert_eq!(placed(&cells, &world.region).await, None, "{status:?}");
        }

        cells.set_status(cell.id(), CellStatus::Open).await?;
        assert_eq!(placed(&cells, &world.region).await, Some(cell.id()));
        Ok(())
    })
    .await;
}

/// @spec-fpr-10
#[tokio::test]
async fn set_status_refuses_full_and_an_unknown_cell() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let cell = open_cell(&cells, &world, &world.region, 5, 0).await?;
        let unknown = CellId(Uuid::new_v4());

        assert!(matches!(
            cells.set_status(cell, CellStatus::Full).await,
            Err(CoreError::Cell(CellError::InvalidTransition))
        ));
        assert_eq!(
            cells.find(cell).await?.expect("exists").status(),
            CellStatus::Open
        );
        assert!(matches!(
            cells.set_status(unknown, CellStatus::Open).await,
            Err(CoreError::Cell(CellError::UnknownCell { id })) if id == unknown.0
        ));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn create_refuses_a_cell_that_is_not_provisioning() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let open = Cell::restore(
            CellId(Uuid::new_v4()),
            world.region.clone(),
            world.dataplane,
            world.instance,
            CellStatus::Open,
            5,
            0,
            Utc::now(),
        )
        .expect("a consistent cell");

        assert!(matches!(
            cells.create(&open).await,
            Err(CoreError::Cell(CellError::InvalidTransition))
        ));
        assert_eq!(cells.find(open.id()).await?, None);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_cell_round_trips_and_the_list_is_ordered_by_age() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let region = Region::new(format!("round-trip-{}", Uuid::new_v4()));

        let younger = provisioning(&world, &region, 7, 5);
        let older = provisioning(&world, &region, 9, 2);
        cells.create(&younger).await?;
        cells.create(&older).await?;

        assert_eq!(cells.find(younger.id()).await?, Some(younger.clone()));
        assert_eq!(cells.find(older.id()).await?, Some(older.clone()));
        assert_eq!(cells.find(CellId(Uuid::new_v4())).await?, None);

        let listed: Vec<Cell> = cells
            .list()
            .await?
            .into_iter()
            .filter(|cell| *cell.region() == region)
            .collect();
        assert_eq!(listed, vec![older, younger]);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_pooled_deployment_round_trips() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);
        let cell = open_cell(&cells, &world, &world.region, 5, 0).await?;

        let tenant = pooled_in(&world, cell, &unique("t"));
        deployments.insert(tenant.clone()).await?;

        let read = deployments.get_by_id(tenant.id).await?.expect("exists");
        assert_eq!(read.distribution, tenant.distribution);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn shared_and_self_hosted_deployments_still_round_trip() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let deployments = PostgresDeploymentRepository::new(&tx);

        for distribution in [Distribution::Shared, Distribution::SelfHosted] {
            let row = deployment(&world, &unique("d"), distribution.clone());
            deployments.insert(row.clone()).await?;
            let read = deployments.get_by_id(row.id).await?.expect("exists");
            assert_eq!(read.distribution, distribution);
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn two_live_pooled_deployments_cannot_share_a_realm_until_the_first_is_deleted() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let cell = open_cell(&cells, &world, &world.region, 5, 0).await?;
        let realm = unique("t");
        let insert = async |name: &str, deleted: bool| -> Result<(), sqlx::Error> {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO deployments \
                 (id, organisation_id, dataplane_id, name, kind, status, namespace, \
                  environment, version, cpu_millis, memory_mib, storage_gib, created_by, \
                  created_at, updated_at, deleted_at, hostname_slug, distribution, \
                  cell_id, realm, cell_slot_held) \
                 VALUES ($1, $2, $3, $4, 'ferriskey', 'successful', $5, 'development', \
                         '26.0.1', 1, 1, 1, $6, now(), now(), \
                         CASE WHEN $7 THEN now() END, $4, 'pooled', $8, $9, true)",
            )
            .bind(Uuid::new_v4())
            .bind(world.organisation.0)
            .bind(world.dataplane.0)
            .bind(name)
            .bind(format!("ns-{name}"))
            .bind(world.user.0)
            .bind(deleted)
            .bind(cell.0)
            .bind(&realm)
            .execute(&mut ***guard)
            .await
            .map(|_| ())
        };

        let first = unique("a");
        insert(&first, false).await.expect("the first realm");
        sqlx::query("SAVEPOINT clash")
            .execute(&mut ***tx.lock().await)
            .await
            .map_err(db_error)?;
        let second = insert(&unique("b"), false).await;
        assert!(second.is_err(), "a live realm cannot be taken twice");
        sqlx::query("ROLLBACK TO SAVEPOINT clash")
            .execute(&mut ***tx.lock().await)
            .await
            .map_err(db_error)?;

        sqlx::query("UPDATE deployments SET deleted_at = now() WHERE name = $1")
            .bind(&first)
            .execute(&mut ***tx.lock().await)
            .await
            .map_err(db_error)?;
        insert(&unique("c"), false)
            .await
            .expect("the realm is free once the first is deleted");
        Ok(())
    })
    .await;
}

async fn refused(tx: &SharedTx<'_>, statement: &str) -> Result<bool, CoreError> {
    sqlx::query("SAVEPOINT attempt")
        .execute(&mut ***tx.lock().await)
        .await
        .map_err(db_error)?;
    let result = sqlx::query(statement)
        .execute(&mut ***tx.lock().await)
        .await;
    sqlx::query("ROLLBACK TO SAVEPOINT attempt")
        .execute(&mut ***tx.lock().await)
        .await
        .map_err(db_error)?;
    Ok(result.is_err())
}

#[tokio::test]
async fn the_database_refuses_rows_that_cannot_be_real() {
    scratch(async |tx| {
        let world = seed_world(&tx).await?;
        let cells = PostgresCellRepository::new(&tx);
        let cell = open_cell(&cells, &world, &world.region, 5, 0).await?;
        let tenant = deployment(&world, &unique("d"), Distribution::Shared);
        PostgresDeploymentRepository::new(&tx)
            .insert(tenant.clone())
            .await?;

        for (what, statement) in [
            (
                "pooled without a cell",
                format!(
                    "UPDATE deployments SET distribution = 'pooled', realm = 'x{}' WHERE id = '{}'",
                    tenant.id.0.simple(),
                    tenant.id.0
                ),
            ),
            (
                "pooled without a realm",
                format!(
                    "UPDATE deployments SET distribution = 'pooled', cell_id = '{}' WHERE id = '{}'",
                    cell.0, tenant.id.0
                ),
            ),
            (
                "a slot held outside a pool",
                format!(
                    "UPDATE deployments SET cell_slot_held = true WHERE id = '{}'",
                    tenant.id.0
                ),
            ),
            (
                "a cell without capacity",
                format!("UPDATE cells SET capacity = 0 WHERE id = '{}'", cell.0),
            ),
            (
                "a cell above 65535",
                format!("UPDATE cells SET capacity = 65536 WHERE id = '{}'", cell.0),
            ),
            (
                "a stored full cell",
                format!("UPDATE cells SET status = 'full' WHERE id = '{}'", cell.0),
            ),
        ] {
            assert!(refused(&tx, &statement).await?, "{what}");
        }
        Ok(())
    })
    .await;
}

struct Committed {
    pool: PgPool,
    world: World,
}

async fn committed_world() -> Option<Committed> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());
    let Some(url) = url else {
        assert!(
            std::env::var("REQUIRE_DATABASE_URL").is_err(),
            "REQUIRE_DATABASE_URL is set but DATABASE_URL is not"
        );
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");
    let world = with_tx(&pool, db_error, async |tx| seed_world(&tx).await)
        .await
        .expect("the world was committed");

    Some(Committed { pool, world })
}

impl Committed {
    async fn clean_up(&self) {
        let region = self.world.region.as_str();
        for statement in [
            "DELETE FROM deployments WHERE cell_id IN (SELECT id FROM cells WHERE region = $1)",
            "DELETE FROM cells WHERE region = $1",
        ] {
            sqlx::query(statement)
                .bind(region)
                .execute(&self.pool)
                .await
                .expect("the test cells were removed");
        }
        for (statement, id) in [
            (
                "DELETE FROM deployments WHERE organisation_id = $1",
                self.world.organisation.0,
            ),
            (
                "DELETE FROM data_planes WHERE id = $1",
                self.world.dataplane.0,
            ),
            (
                "DELETE FROM organisations WHERE id = $1",
                self.world.organisation.0,
            ),
            ("DELETE FROM users WHERE id = $1", self.world.user.0),
        ] {
            sqlx::query(statement)
                .bind(id)
                .execute(&self.pool)
                .await
                .expect("the test data was removed");
        }
    }
}

async fn race_for_a_slot(committed: &Committed, cell_id: CellId) -> Option<CellId> {
    with_tx(&committed.pool, db_error, async |tx| {
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);

        let Some(cell) = cells.place(&committed.world.region).await? else {
            return Ok(None);
        };
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        deployments
            .insert(pooled_in(&committed.world, cell.id(), &unique("t")))
            .await?;
        assert_eq!(cell.id(), cell_id);
        Ok(Some(cell.id()))
    })
    .await
    .expect("the racing transaction committed")
}

/// @spec-fpr-4
#[tokio::test]
async fn two_creations_racing_for_the_last_slot_never_exceed_the_capacity() {
    let Some(committed) = committed_world().await else {
        return;
    };

    let cell_id = with_tx(&committed.pool, db_error, async |tx| {
        let cells = PostgresCellRepository::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);
        let cell = open_cell(&cells, &committed.world, &committed.world.region, 2, 0).await?;
        fill(&deployments, &committed.world, cell, 1).await?;
        Ok(cell)
    })
    .await
    .expect("the cell was committed");

    let (first, second) = tokio::join!(
        race_for_a_slot(&committed, cell_id),
        race_for_a_slot(&committed, cell_id)
    );

    let cells_after = with_tx(&committed.pool, db_error, async |tx| {
        PostgresCellRepository::new(&tx).find(cell_id).await
    })
    .await;
    committed.clean_up().await;
    let cells_after = cells_after
        .expect("the cell was read")
        .expect("the cell exists");

    let winners = [first, second].into_iter().flatten().count();
    assert_eq!(winners, 1, "exactly one creation gets the last slot");
    assert_eq!(cells_after.realms(), 2);
    assert_eq!(cells_after.status(), CellStatus::Full);
}
