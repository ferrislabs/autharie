use std::time::Duration;

use autharie_domain::{
    CoreError,
    dataplane::{ports::DataPlaneRepository, value_objects::DataPlaneId},
    deployments::DeploymentId,
};
use autharie_persistence::with_tx;
use autharie_postgres::dataplane::PostgresDataPlaneRepository;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::{sync::oneshot, time::timeout};
use uuid::Uuid;

const PATIENCE: Duration = Duration::from_millis(400);

async fn pool() -> Option<PgPool> {
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
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but the database is unreachable"),
    )
}

fn map_err(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

#[tokio::test]
async fn a_second_reader_of_a_plane_load_waits_for_the_first_to_commit_and_another_plane_does_not()
{
    let Some(pool) = pool().await else {
        return;
    };
    let plane = DataPlaneId(Uuid::new_v4());
    let other = DataPlaneId(Uuid::new_v4());
    let resized = DeploymentId(Uuid::new_v4());

    let (locked_tx, locked_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();

    let first = tokio::spawn({
        let pool = pool.clone();
        async move {
            with_tx(&pool, map_err, async |tx| {
                let load = PostgresDataPlaneRepository::new(&tx)
                    .load_excluding(&plane, &resized)
                    .await?;
                locked_tx.send(()).expect("the test is listening");
                release_rx.await.expect("the test lets go");

                Ok(load)
            })
            .await
        }
    });
    locked_rx.await.expect("the first reader holds the plane");

    let mut second = tokio::spawn({
        let pool = pool.clone();
        async move {
            with_tx(&pool, map_err, async |tx| {
                PostgresDataPlaneRepository::new(&tx)
                    .load_excluding(&plane, &DeploymentId(Uuid::new_v4()))
                    .await
            })
            .await
        }
    });

    assert!(
        timeout(PATIENCE, &mut second).await.is_err(),
        "the second reader answered while the first still held the plane"
    );

    timeout(
        PATIENCE,
        with_tx(&pool, map_err, async |tx| {
            PostgresDataPlaneRepository::new(&tx)
                .load_excluding(&other, &resized)
                .await
        }),
    )
    .await
    .expect("another plane is not held up")
    .expect("the other plane answered");

    release_tx.send(()).expect("the first reader is waiting");
    first
        .await
        .expect("the first reader ran")
        .expect("the first reader committed");
    second
        .await
        .expect("the second reader ran")
        .expect("the second reader answered");
}
