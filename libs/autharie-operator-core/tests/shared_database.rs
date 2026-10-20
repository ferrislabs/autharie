use std::time::{SystemTime, UNIX_EPOCH};

use autharie_operator_core::domain::shared_database::SharedDatabaseName;
use autharie_operator_core::infrastructure::shared_database::{
    ROLE_CONNECTION_LIMIT, drop_database, provision,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, Executor, PgPool};
use uuid::Uuid;

async fn superuser_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());

    let Some(url) = url else {
        assert!(
            std::env::var("REQUIRE_DATABASE_URL").is_err(),
            "REQUIRE_DATABASE_URL is set but DATABASE_URL is not: these tests \
             would have skipped and reported success"
        );
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };

    Some(
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but the database is unreachable"),
    )
}

fn unique_name() -> SharedDatabaseName {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    SharedDatabaseName::for_deployment(Uuid::from_u128(nanos ^ Uuid::new_v4().as_u128()))
}

fn options_as(
    admin: &PgPool,
    role: &SharedDatabaseName,
    password: &str,
    database: &SharedDatabaseName,
) -> PgConnectOptions {
    admin
        .connect_options()
        .as_ref()
        .clone()
        .username(role.as_str())
        .password(password)
        .database(database.as_str())
}

async fn count(admin: &PgPool, sql: &str, name: &SharedDatabaseName) -> i64 {
    sqlx::query_scalar(sql)
        .bind(name.as_str())
        .fetch_one(admin)
        .await
        .unwrap()
}

const DATABASES: &str = "SELECT count(*) FROM pg_database WHERE datname = $1";
const ROLES: &str = "SELECT count(*) FROM pg_roles WHERE rolname = $1";

#[tokio::test]
async fn provisioning_twice_changes_nothing_and_keeps_the_latest_password() {
    let Some(admin) = superuser_pool().await else {
        return;
    };
    let name = unique_name();

    provision(&admin, &name, "first").await.unwrap();
    provision(&admin, &name, "second").await.unwrap();

    assert_eq!(count(&admin, DATABASES, &name).await, 1);
    assert_eq!(count(&admin, ROLES, &name).await, 1);

    let mut connection = options_as(&admin, &name, "second", &name)
        .connect()
        .await
        .unwrap();
    connection.execute("SELECT 1").await.unwrap();
    assert!(
        options_as(&admin, &name, "first", &name)
            .connect()
            .await
            .is_err()
    );

    drop_database(&admin, &name).await.unwrap();
}

#[tokio::test]
async fn the_role_is_capped_and_converges_to_the_cap() {
    let Some(admin) = superuser_pool().await else {
        return;
    };
    let name = unique_name();

    provision(&admin, &name, "pw").await.unwrap();
    sqlx::query(&format!(
        "ALTER ROLE \"{}\" CONNECTION LIMIT 500",
        name.as_str()
    ))
    .execute(&admin)
    .await
    .unwrap();
    provision(&admin, &name, "pw").await.unwrap();

    let limit: i32 = sqlx::query_scalar("SELECT rolconnlimit FROM pg_roles WHERE rolname = $1")
        .bind(name.as_str())
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(limit, ROLE_CONNECTION_LIMIT as i32);
    assert_eq!(limit, 10);

    drop_database(&admin, &name).await.unwrap();
}

#[tokio::test]
async fn one_role_cannot_connect_to_another_roles_database() {
    let Some(admin) = superuser_pool().await else {
        return;
    };
    let a = unique_name();
    let b = unique_name();

    provision(&admin, &a, "pw-a").await.unwrap();
    provision(&admin, &b, "pw-b").await.unwrap();

    assert!(options_as(&admin, &a, "pw-a", &a).connect().await.is_ok());
    assert!(options_as(&admin, &b, "pw-b", &b).connect().await.is_ok());
    assert!(options_as(&admin, &a, "pw-a", &b).connect().await.is_err());
    assert!(options_as(&admin, &b, "pw-b", &a).connect().await.is_err());

    drop_database(&admin, &a).await.unwrap();
    drop_database(&admin, &b).await.unwrap();
}

#[tokio::test]
async fn dropping_removes_only_that_database_and_role() {
    let Some(admin) = superuser_pool().await else {
        return;
    };
    let a = unique_name();
    let b = unique_name();

    provision(&admin, &a, "pw-a").await.unwrap();
    provision(&admin, &b, "pw-b").await.unwrap();
    let held = options_as(&admin, &a, "pw-a", &a).connect().await.unwrap();

    drop_database(&admin, &a).await.unwrap();
    drop(held);

    assert_eq!(count(&admin, DATABASES, &a).await, 0);
    assert_eq!(count(&admin, ROLES, &a).await, 0);
    assert_eq!(count(&admin, DATABASES, &b).await, 1);
    assert_eq!(count(&admin, ROLES, &b).await, 1);
    assert!(options_as(&admin, &b, "pw-b", &b).connect().await.is_ok());

    drop_database(&admin, &a).await.unwrap();
    drop_database(&admin, &b).await.unwrap();
}
