use std::fmt::Display;

use sqlx::{
    PgPool,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

mod support;
use support::pool;

const BEFORE_BILLING: i64 = 20261021000001;

fn text(error: impl Display) -> String {
    error.to_string()
}

#[derive(Debug, PartialEq, Eq)]
struct Held {
    tiers: Vec<String>,
    subscriptions: i64,
    standard: Vec<String>,
    sandbox: Vec<String>,
    private: Vec<String>,
    deleted: i64,
    standard_terms: (i64, String, i64),
    sandbox_owner_matches: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Observed {
    billing_after_undo: bool,
    deployments_after_undo: i64,
    held: Held,
    billing_after_second_undo: bool,
    deployments_after_second_undo: i64,
    extension_after_second_undo: bool,
}

async fn has_billing(pool: &PgPool) -> Result<bool, String> {
    sqlx::query_scalar("SELECT to_regclass('billing_subscriptions') IS NOT NULL")
        .fetch_one(pool)
        .await
        .map_err(text)
}

async fn deployments(pool: &PgPool) -> Result<i64, String> {
    sqlx::query_scalar("SELECT count(*) FROM deployments")
        .fetch_one(pool)
        .await
        .map_err(text)
}

struct Fixture {
    organisation: Uuid,
    standard: Uuid,
    sandbox: Uuid,
    private: Uuid,
    deleted: Uuid,
}

async fn insert_fixture(pool: &PgPool) -> Result<Fixture, String> {
    let user = Uuid::new_v4();
    let organisation = Uuid::new_v4();
    let shared_plane = Uuid::new_v4();
    let dedicated_plane = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'migration', $3)")
        .bind(user)
        .bind(format!("{user}@migration.test"))
        .bind(user.to_string())
        .execute(pool)
        .await
        .map_err(text)?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, max_storage_gb) \
         VALUES ($1, 'migration', $2, $3, 'active', 'free', 10, 10, 10)",
    )
    .bind(organisation)
    .bind(organisation.to_string())
    .bind(user)
    .execute(pool)
    .await
    .map_err(text)?;

    sqlx::query(
        "INSERT INTO data_planes \
         (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
          capacity_storage_gib) \
         VALUES ($1, 'shared', 'migration-shared', 'active', 8000, 16384, 200)",
    )
    .bind(shared_plane)
    .execute(pool)
    .await
    .map_err(text)?;

    sqlx::query(
        "INSERT INTO data_planes \
         (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
          capacity_storage_gib, organisation_id) \
         VALUES ($1, 'dedicated', 'migration-dedicated', 'active', 8000, 16384, 200, $2)",
    )
    .bind(dedicated_plane)
    .bind(organisation)
    .execute(pool)
    .await
    .map_err(text)?;

    let standard = Uuid::new_v4();
    let sandbox = Uuid::new_v4();
    let private = Uuid::new_v4();
    let deleted = Uuid::new_v4();

    for (id, plane, offer, deleted_at) in [
        (standard, shared_plane, Some("standard"), false),
        (sandbox, shared_plane, None, false),
        (private, dedicated_plane, None, false),
        (deleted, shared_plane, Some("standard"), true),
    ] {
        sqlx::query(
            "INSERT INTO deployments \
             (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
              created_by, cpu_millis, memory_mib, storage_gib, hostname_slug, offer, \
              deleted_at) \
             VALUES ($1, $2, $3, $4, 'ferriskey', 'successful', 'migration-test', '26.0.1', \
                     $5, 250, 512, 1, $4, $6, CASE WHEN $7 THEN now() END)",
        )
        .bind(id)
        .bind(organisation)
        .bind(plane)
        .bind(format!("migration-{id}"))
        .bind(user)
        .bind(offer)
        .bind(deleted_at)
        .execute(pool)
        .await
        .map_err(text)?;
    }

    Ok(Fixture {
        organisation,
        standard,
        sandbox,
        private,
        deleted,
    })
}

async fn valid_now(pool: &PgPool, deployment: Uuid) -> Result<Vec<String>, String> {
    sqlx::query_scalar(
        "SELECT tier_code FROM billing_subscriptions \
         WHERE deployment_id = $1 AND validity @> now() ORDER BY tier_code",
    )
    .bind(deployment)
    .fetch_all(pool)
    .await
    .map_err(text)
}

async fn held(pool: &PgPool, fixture: &Fixture) -> Result<Held, String> {
    let tiers: Vec<String> = sqlx::query_scalar("SELECT code FROM billing_tiers ORDER BY code")
        .fetch_all(pool)
        .await
        .map_err(text)?;

    let subscriptions: i64 = sqlx::query_scalar("SELECT count(*) FROM billing_subscriptions")
        .fetch_one(pool)
        .await
        .map_err(text)?;

    let deleted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM billing_subscriptions WHERE deployment_id = $1")
            .bind(fixture.deleted)
            .fetch_one(pool)
            .await
            .map_err(text)?;

    let standard_terms: (i64, String, i64) = sqlx::query_as(
        "SELECT monthly_price_minor, offer, cardinality(features)::bigint \
         FROM billing_subscriptions WHERE deployment_id = $1",
    )
    .bind(fixture.standard)
    .fetch_one(pool)
    .await
    .map_err(text)?;

    let sandbox_owner_matches: bool = sqlx::query_scalar(
        "SELECT organisation_id = $2 AND ends_at IS NULL AND starts_at <= now() \
         FROM billing_subscriptions WHERE deployment_id = $1",
    )
    .bind(fixture.sandbox)
    .bind(fixture.organisation)
    .fetch_one(pool)
    .await
    .map_err(text)?;

    Ok(Held {
        tiers,
        subscriptions,
        standard: valid_now(pool, fixture.standard).await?,
        sandbox: valid_now(pool, fixture.sandbox).await?,
        private: valid_now(pool, fixture.private).await?,
        deleted,
        standard_terms,
        sandbox_owner_matches,
    })
}

async fn walk(migrator: &Migrator, pool: &PgPool) -> Result<Observed, String> {
    migrator.run(pool).await.map_err(text)?;
    migrator.undo(pool, BEFORE_BILLING).await.map_err(text)?;

    let billing_after_undo = has_billing(pool).await?;
    let fixture = insert_fixture(pool).await?;
    let deployments_after_undo = deployments(pool).await?;

    migrator.run(pool).await.map_err(text)?;
    let held = held(pool, &fixture).await?;

    migrator.undo(pool, BEFORE_BILLING).await.map_err(text)?;
    let extension_after_second_undo: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'btree_gist')",
    )
    .fetch_one(pool)
    .await
    .map_err(text)?;

    Ok(Observed {
        billing_after_undo,
        deployments_after_undo,
        held,
        billing_after_second_undo: has_billing(pool).await?,
        deployments_after_second_undo: deployments(pool).await?,
        extension_after_second_undo,
    })
}

#[tokio::test]
async fn the_billing_migration_applies_backfills_and_reverts_with_data_present() {
    let Some(admin) = pool().await else {
        return;
    };

    let name = format!("billing_migration_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("a scratch database can be created");

    let options: PgConnectOptions = (*admin.connect_options()).clone().database(&name);
    let scratch = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("the scratch database is reachable");

    let migrator = sqlx::migrate!("../autharie-core/migrations");
    let observed = walk(&migrator, &scratch).await;

    scratch.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .expect("the scratch database can be dropped");

    let observed = observed.expect("the migrations walked");

    assert!(
        !observed.billing_after_undo,
        "undo removes the billing tables"
    );
    assert_eq!(observed.deployments_after_undo, 4);

    let held = observed.held;
    assert_eq!(
        held.tiers,
        [
            "legacy-free",
            "legacy-private",
            "legacy-sandbox",
            "legacy-scale",
            "legacy-standard"
        ]
    );
    assert_eq!(held.subscriptions, 3, "one per live deployment");
    assert_eq!(held.standard, ["legacy-standard"]);
    assert_eq!(held.sandbox, ["legacy-sandbox"]);
    assert_eq!(held.private, ["legacy-private"]);
    assert_eq!(held.deleted, 0, "a deleted deployment is not billed");
    assert_eq!(held.standard_terms, (0, "standard".to_string(), 8));
    assert!(held.sandbox_owner_matches);

    assert!(
        !observed.billing_after_second_undo,
        "undo works with data present"
    );
    assert_eq!(
        observed.deployments_after_second_undo, 4,
        "undo leaves the deployments alone"
    );
    assert!(!observed.extension_after_second_undo);
}
