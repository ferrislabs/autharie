use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args, handlers::deployments::availability::get_deployment_availability_handler,
    state::AppState,
};
use autharie_auth::{Identity, User};
use autharie_core::AutharieService;
use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use axum_extra::routing::RouterExt;
use chrono::{Duration as Days, Utc};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
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

    Some(
        PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but the database is unreachable"),
    )
}

struct Estate {
    tag: String,
    owner: Uuid,
    organisation: Uuid,
    stranger: Uuid,
    other_organisation: Uuid,
    data_plane: Uuid,
    deployment: Uuid,
    other_deployment: Uuid,
}

impl Estate {
    fn new() -> Self {
        Self {
            tag: format!("avail-{}", Uuid::new_v4().simple()),
            owner: Uuid::new_v4(),
            organisation: Uuid::new_v4(),
            stranger: Uuid::new_v4(),
            other_organisation: Uuid::new_v4(),
            data_plane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            other_deployment: Uuid::new_v4(),
        }
    }

    fn uri(&self, query: &str) -> String {
        format!(
            "/organisations/{}/deployments/{}/availability{query}",
            self.organisation, self.deployment
        )
    }
}

async fn seed(pool: &PgPool, estate: &Estate) {
    for user in [estate.owner, estate.stranger] {
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, $3, $4)")
            .bind(user)
            .bind(format!("{user}@availability.test"))
            .bind(&estate.tag)
            .bind(user.to_string())
            .execute(pool)
            .await
            .expect("a user");
    }
    for (organisation, owner) in [
        (estate.organisation, estate.owner),
        (estate.other_organisation, estate.stranger),
    ] {
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, $2, $1::text, $3, 'active', 'free', 5, 5, 5, now(), now())",
        )
        .bind(organisation)
        .bind(&estate.tag)
        .bind(owner)
        .execute(pool)
        .await
        .expect("an organisation");
    }
    sqlx::query(
        "INSERT INTO data_planes \
         (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
          capacity_storage_gib, created_at, updated_at) \
         VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200, now(), now())",
    )
    .bind(estate.data_plane)
    .bind(&estate.tag)
    .execute(pool)
    .await
    .expect("a data plane");
    for (deployment, organisation, owner) in [
        (estate.deployment, estate.organisation, estate.owner),
        (
            estate.other_deployment,
            estate.other_organisation,
            estate.stranger,
        ),
    ] {
        sqlx::query(
            "INSERT INTO deployments \
             (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
              cpu_millis, memory_mib, storage_gib, created_by, created_at, updated_at, \
              hostname_slug) \
             VALUES ($1, $2, $3, 'auth', 'ferriskey', 'successful', $1::text, '26.0.0', \
                     500, 1024, 1, $4, now(), now(), $1::text)",
        )
        .bind(deployment)
        .bind(organisation)
        .bind(estate.data_plane)
        .bind(owner)
        .execute(pool)
        .await
        .expect("a deployment");
    }
}

async fn record(pool: &PgPool, deployment: Uuid, days_ago: i64, checks: i32, failed: i32) {
    let day = (Utc::now() - Days::days(days_ago)).date_naive();
    sqlx::query(
        "INSERT INTO deployment_reachability_checks (deployment_id, checked_at, reachable) \
         SELECT $1, ($2::date + time '00:00' + n * interval '1 minute') AT TIME ZONE 'UTC', \
                n >= $4 \
         FROM generate_series(0, $3 - 1) AS n",
    )
    .bind(deployment)
    .bind(day)
    .bind(checks)
    .bind(failed)
    .execute(pool)
    .await
    .expect("checks");
}

async fn forget(pool: &PgPool, estate: &Estate) {
    for (statement, id) in [
        (
            "DELETE FROM deployment_reachability_checks WHERE deployment_id = $1",
            estate.deployment,
        ),
        ("DELETE FROM deployments WHERE id = $1", estate.deployment),
        (
            "DELETE FROM deployments WHERE id = $1",
            estate.other_deployment,
        ),
        ("DELETE FROM data_planes WHERE id = $1", estate.data_plane),
        (
            "DELETE FROM organisations WHERE id = $1",
            estate.organisation,
        ),
        (
            "DELETE FROM organisations WHERE id = $1",
            estate.other_organisation,
        ),
        ("DELETE FROM users WHERE id = $1", estate.owner),
        ("DELETE FROM users WHERE id = $1", estate.stranger),
    ] {
        sqlx::query(statement)
            .bind(id)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

fn app(pool: PgPool, user: Uuid) -> Router {
    let state = AppState {
        args: Arc::new(Args::default()),
        service: AutharieService::new(pool),
        certificate_source: None,
        quickwit_search: None,
        quickwit_traces: None,
        billing: None,
    };

    Router::new()
        .typed_get(get_deployment_availability_handler)
        .layer(Extension(Identity::User(User {
            scopes: Vec::new(),
            act: None,
            id: user.to_string(),
            username: "someone".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })))
        .with_state(state)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .expect("a request");
    let response = app.clone().oneshot(request).await.expect("a response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");

    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_member_reads_ninety_days_by_default_with_the_documented_shape() {
    let Some(pool) = pool().await else { return };
    let estate = Estate::new();
    seed(&pool, &estate).await;
    record(&pool, estate.deployment, 2, 100, 0).await;
    record(&pool, estate.deployment, 0, 100, 3).await;

    let (status, body) = get(&app(pool.clone(), estate.owner), &estate.uri("")).await;
    let (_, week) = get(&app(pool.clone(), estate.owner), &estate.uri("?days=7")).await;
    forget(&pool, &estate).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let data = &body["data"];
    let days = data["days"].as_array().expect("days");
    assert_eq!(days.len(), 90);
    assert_eq!(data["probe_interval_seconds"], 60);
    assert_eq!(
        days[89]["day"],
        Utc::now().date_naive().to_string().as_str()
    );
    assert_eq!(days[89]["checks"], 100);
    assert_eq!(days[89]["failed"], 3);
    assert_eq!(days[89]["down_seconds"], 180);
    assert_eq!(days[89]["uptime"], 0.97);
    assert_eq!(days[88]["checks"], 0);
    assert_eq!(days[88]["uptime"], Value::Null);
    assert_eq!(days[87]["uptime"], 1.0);
    assert_eq!(data["uptime"], 197.0 / 200.0);
    assert_eq!(week["data"]["days"].as_array().expect("days").len(), 7);
}

#[tokio::test]
async fn a_deployment_without_checks_has_no_uptime() {
    let Some(pool) = pool().await else { return };
    let estate = Estate::new();
    seed(&pool, &estate).await;

    let (status, body) = get(&app(pool.clone(), estate.owner), &estate.uri("?days=1")).await;
    forget(&pool, &estate).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["uptime"], Value::Null);
    assert_eq!(body["data"]["days"][0]["checks"], 0);
}

#[tokio::test]
async fn days_out_of_range_are_a_bad_request() {
    let Some(pool) = pool().await else { return };
    let estate = Estate::new();
    seed(&pool, &estate).await;

    let app = app(pool.clone(), estate.owner);
    let mut statuses = Vec::new();
    for query in ["?days=0", "?days=91", "?days=-1"] {
        statuses.push((query, get(&app, &estate.uri(query)).await.0));
    }
    forget(&pool, &estate).await;

    for (query, status) in statuses {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
    }
}

#[tokio::test]
async fn a_non_member_is_refused_and_another_organisations_deployment_is_not_found() {
    let Some(pool) = pool().await else { return };
    let estate = Estate::new();
    seed(&pool, &estate).await;

    let (outsider, _) = get(&app(pool.clone(), estate.stranger), &estate.uri("")).await;
    let foreign = format!(
        "/organisations/{}/deployments/{}/availability",
        estate.organisation, estate.other_deployment
    );
    let (cross, _) = get(&app(pool.clone(), estate.owner), &foreign).await;
    forget(&pool, &estate).await;

    assert_eq!(outsider, StatusCode::FORBIDDEN);
    assert_eq!(cross, StatusCode::NOT_FOUND);
}
