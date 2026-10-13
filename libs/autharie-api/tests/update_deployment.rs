use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::deployments::{
        get_deployment::get_deployment_handler, update_deployment::update_deployment_handler,
    },
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
use serde_json::{Value, json};
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

struct Fixture {
    tag: String,
    owner: Uuid,
    organisation: Uuid,
    data_plane: Uuid,
    deployment: Uuid,
}

impl Fixture {
    fn new() -> Self {
        Self {
            tag: format!("patch-{}", Uuid::new_v4().simple()),
            owner: Uuid::new_v4(),
            organisation: Uuid::new_v4(),
            data_plane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
        }
    }

    fn uri(&self) -> String {
        format!(
            "/organisations/{}/deployments/{}",
            self.organisation, self.deployment
        )
    }
}

async fn seed_failed_deployment(pool: &PgPool, fixture: &Fixture) {
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, $3, $4)")
        .bind(fixture.owner)
        .bind(format!("{}@patch-deployment.test", fixture.owner))
        .bind(&fixture.tag)
        .bind(fixture.owner.to_string())
        .execute(pool)
        .await
        .expect("a user");
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, $2, $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
    )
    .bind(fixture.organisation)
    .bind(&fixture.tag)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("an organisation");
    sqlx::query(
        "INSERT INTO data_planes \
         (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
          capacity_storage_gib, created_at, updated_at) \
         VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200, now(), now())",
    )
    .bind(fixture.data_plane)
    .bind(&fixture.tag)
    .execute(pool)
    .await
    .expect("a data plane");
    sqlx::query(
        "INSERT INTO deployments \
         (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
          cpu_millis, memory_mib, storage_gib, created_by, created_at, updated_at, hostname_slug) \
         VALUES ($1, $2, $3, 'auth', 'ferriskey', 'failed', $4, '26.0.0', \
                 500, 1024, 1, $5, now(), now(), $4)",
    )
    .bind(fixture.deployment)
    .bind(fixture.organisation)
    .bind(fixture.data_plane)
    .bind(&fixture.tag)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("a deployment");
}

async fn forget(pool: &PgPool, fixture: &Fixture) {
    for (statement, id) in [
        ("DELETE FROM deployments WHERE id = $1", fixture.deployment),
        ("DELETE FROM data_planes WHERE id = $1", fixture.data_plane),
        (
            "DELETE FROM organisations WHERE id = $1",
            fixture.organisation,
        ),
        ("DELETE FROM users WHERE id = $1", fixture.owner),
    ] {
        sqlx::query(statement)
            .bind(id)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

fn app(pool: PgPool, owner: Uuid) -> Router {
    let state = AppState {
        args: Arc::new(Args::default()),
        service: AutharieService::new(pool),
        certificate_source: None,
        quickwit_search: None,
        quickwit_traces: None,
    };

    Router::new()
        .typed_get(get_deployment_handler)
        .typed_patch(update_deployment_handler)
        .layer(Extension(Identity::User(User {
            id: owner.to_string(),
            username: "owner".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })))
        .with_state(state)
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
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

fn platform_record(deployment: &Value) -> Value {
    let deployment = deployment.get("data").unwrap_or(deployment);
    json!({
        "status": deployment["status"],
        "namespace": deployment["namespace"],
        "kind": deployment["kind"],
        "version": deployment["version"],
        "deployed_at": deployment["deployed_at"],
    })
}

#[tokio::test]
async fn a_customer_patch_cannot_rewrite_the_platform_record() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed_failed_deployment(&pool, &fixture).await;
    let app = app(pool.clone(), fixture.owner);
    let (_, before) = call(&app, Method::GET, &fixture.uri(), None).await;

    let mut refusals = Vec::new();
    for body in [
        json!({"status": "successful"}),
        json!({"namespace": "someone-else"}),
        json!({"kind": "keycloak"}),
        json!({"version": "99.0.0"}),
        json!({"deployed_at": "2026-01-01T00:00:00Z"}),
        json!({"name": "renamed", "status": "successful"}),
    ] {
        let (status, response) =
            call(&app, Method::PATCH, &fixture.uri(), Some(body.clone())).await;
        refusals.push((body, status, response));
    }
    let (_, after) = call(&app, Method::GET, &fixture.uri(), None).await;
    forget(&pool, &fixture).await;

    for (body, status, response) in refusals {
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body} -> {response}"
        );
        assert_eq!(response["code"], "E_UNPROCESSABLE", "{body} -> {response}");
    }
    assert_eq!(platform_record(&before)["status"], "failed", "{before}");
    assert_eq!(platform_record(&after), platform_record(&before));
    assert_eq!(
        after.get("data").unwrap_or(&after)["name"],
        "auth",
        "{after}"
    );
}

#[tokio::test]
async fn a_customer_patch_still_renames_and_switches_log_shipping() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed_failed_deployment(&pool, &fixture).await;
    let app = app(pool.clone(), fixture.owner);
    let (_, before) = call(&app, Method::GET, &fixture.uri(), None).await;

    let (status, updated) = call(
        &app,
        Method::PATCH,
        &fixture.uri(),
        Some(json!({"name": "renamed", "log_shipping_enabled": true})),
    )
    .await;
    let (_, after) = call(&app, Method::GET, &fixture.uri(), None).await;
    forget(&pool, &fixture).await;

    assert_eq!(status, StatusCode::OK, "{updated}");
    let after_data = after.get("data").unwrap_or(&after);
    assert_eq!(after_data["name"], "renamed", "{after}");
    assert_eq!(after_data["log_shipping_enabled"], true, "{after}");
    assert_eq!(platform_record(&after), platform_record(&before));
}
