use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::deployments::power::{start_deployment_handler, stop_deployment_handler},
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

struct Fixture {
    tag: String,
    owner: Uuid,
    stranger: Uuid,
    organisation: Uuid,
    data_plane: Uuid,
    deployment: Uuid,
    statement: Uuid,
}

impl Fixture {
    fn new() -> Self {
        Self {
            tag: format!("power-{}", Uuid::new_v4().simple()),
            owner: Uuid::new_v4(),
            stranger: Uuid::new_v4(),
            organisation: Uuid::new_v4(),
            data_plane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            statement: Uuid::new_v4(),
        }
    }

    fn uri(&self, verb: &str) -> String {
        format!(
            "/organisations/{}/deployments/{}/{verb}",
            self.organisation, self.deployment
        )
    }
}

async fn seed(pool: &PgPool, fixture: &Fixture, status: &str) {
    for user in [fixture.owner, fixture.stranger] {
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, $3, $4)")
            .bind(user)
            .bind(format!("{user}@deployment-power.test"))
            .bind(&fixture.tag)
            .bind(user.to_string())
            .execute(pool)
            .await
            .expect("a user");
    }
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
         VALUES ($1, $2, $3, $4, 'ferriskey', $6, $4, '26.0.0', \
                 500, 1024, 1, $5, now(), now(), $4)",
    )
    .bind(fixture.deployment)
    .bind(fixture.organisation)
    .bind(fixture.data_plane)
    .bind(&fixture.tag)
    .bind(fixture.owner)
    .bind(status)
    .execute(pool)
    .await
    .expect("a deployment");
}

async fn suspend_billing(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO billing_statements (id, organisation_id, period, status, total_minor) \
         VALUES ($1, $2, '2026-09', 'payment_failed', 1000)",
    )
    .bind(fixture.statement)
    .bind(fixture.organisation)
    .execute(pool)
    .await
    .expect("a statement");
    sqlx::query(
        "INSERT INTO billing_suspensions \
         (id, organisation_id, statement_id, suspended_organisation, suspended_at) \
         VALUES ($1, $2, $3, false, now())",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.organisation)
    .bind(fixture.statement)
    .execute(pool)
    .await
    .expect("a suspension");
}

async fn lift_billing_suspension(pool: &PgPool, fixture: &Fixture) {
    sqlx::query("UPDATE billing_suspensions SET lifted_at = now() WHERE organisation_id = $1")
        .bind(fixture.organisation)
        .execute(pool)
        .await
        .expect("a lifted suspension");
}

/// What Herald does: takes what is pending and hands it to the data plane.
/// There is none in this test, so an order stays on its way until this runs.
async fn deliver(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "UPDATE actions SET status = 'published', status_at = now(), leased_until = NULL \
         WHERE deployment_id = $1 AND status IN ('pending', 'leased', 'pulled')",
    )
    .bind(fixture.deployment)
    .execute(pool)
    .await
    .expect("delivered orders");
}

async fn forget(pool: &PgPool, fixture: &Fixture) {
    for (statement, id) in [
        (
            "DELETE FROM actions WHERE deployment_id = $1",
            fixture.deployment,
        ),
        (
            "DELETE FROM billing_suspensions WHERE organisation_id = $1",
            fixture.organisation,
        ),
        (
            "DELETE FROM billing_statements WHERE organisation_id = $1",
            fixture.organisation,
        ),
        ("DELETE FROM deployments WHERE id = $1", fixture.deployment),
        ("DELETE FROM data_planes WHERE id = $1", fixture.data_plane),
        (
            "DELETE FROM organisations WHERE id = $1",
            fixture.organisation,
        ),
        ("DELETE FROM users WHERE id = $1", fixture.owner),
        ("DELETE FROM users WHERE id = $1", fixture.stranger),
    ] {
        sqlx::query(statement)
            .bind(id)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

fn app(pool: PgPool, caller: Uuid) -> Router {
    let state = AppState {
        args: Arc::new(Args::default()),
        service: AutharieService::new(pool),
        certificate_source: None,
        quickwit_search: None,
        quickwit_traces: None,
        billing: None,
    };

    Router::new()
        .typed_post(stop_deployment_handler)
        .typed_post(start_deployment_handler)
        .layer(Extension(Identity::User(User {
            id: caller.to_string(),
            username: "caller".to_string(),
            email: None,
            name: None,
            roles: vec![],
            scopes: vec![],
            act: None,
        })))
        .with_state(state)
}

async fn post(app: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
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

async fn status_of(pool: &PgPool, fixture: &Fixture) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM deployments WHERE id = $1")
        .bind(fixture.deployment)
        .fetch_one(pool)
        .await
        .expect("a status")
}

async fn recorded(pool: &PgPool, fixture: &Fixture) -> Vec<(String, String, Option<Uuid>)> {
    sqlx::query_as::<_, (String, String, Option<Uuid>)>(
        "SELECT action_type, source_type, source_user_id FROM actions \
         WHERE deployment_id = $1 ORDER BY created_at, id",
    )
    .bind(fixture.deployment)
    .fetch_all(pool)
    .await
    .expect("the recorded actions")
}

#[tokio::test]
async fn an_owner_stops_a_running_deployment_and_starts_it_again() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    let app = app(pool.clone(), fixture.owner);

    let (stopped, stop_body) = post(&app, &fixture.uri("stop")).await;
    let after_stop = status_of(&pool, &fixture).await;
    deliver(&pool, &fixture).await;
    let (started, start_body) = post(&app, &fixture.uri("start")).await;
    let after_start = status_of(&pool, &fixture).await;
    let actions = recorded(&pool, &fixture).await;
    forget(&pool, &fixture).await;

    assert_eq!(stopped, StatusCode::OK, "{stop_body}");
    assert_eq!(stop_body["data"]["status"], "stopped", "{stop_body}");
    assert_eq!(after_stop, "stopped");
    assert_eq!(started, StatusCode::OK, "{start_body}");
    assert_eq!(start_body["data"]["status"], "in_progress", "{start_body}");
    assert_eq!(after_start, "in_progress");
    assert_eq!(
        actions
            .iter()
            .map(|(kind, _, _)| kind.as_str())
            .collect::<Vec<_>>(),
        ["deployment.stop", "deployment.start"]
    );
    for (_, source, user) in &actions {
        assert_eq!(source, "user");
        assert_eq!(*user, Some(fixture.owner));
    }
}

#[tokio::test]
async fn stopping_twice_and_starting_what_runs_are_conflicts_that_name_the_state() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    let app = app(pool.clone(), fixture.owner);

    let (_, _) = post(&app, &fixture.uri("stop")).await;
    deliver(&pool, &fixture).await;
    let (stop_again, stop_again_body) = post(&app, &fixture.uri("stop")).await;
    let (_, _) = post(&app, &fixture.uri("start")).await;
    deliver(&pool, &fixture).await;
    let (start_again, start_again_body) = post(&app, &fixture.uri("start")).await;
    let actions = recorded(&pool, &fixture).await;
    forget(&pool, &fixture).await;

    assert_eq!(stop_again, StatusCode::CONFLICT, "{stop_again_body}");
    assert!(
        stop_again_body["message"]
            .as_str()
            .is_some_and(|message| message.contains("stopped")),
        "{stop_again_body}"
    );
    assert_eq!(start_again, StatusCode::CONFLICT, "{start_again_body}");
    assert!(
        start_again_body["message"]
            .as_str()
            .is_some_and(|message| message.contains("in_progress")),
        "{start_again_body}"
    );
    assert_eq!(actions.len(), 2, "a refused request records nothing");
}

#[tokio::test]
async fn a_deployment_that_has_not_started_has_nothing_to_stop() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "pending").await;
    let app = app(pool.clone(), fixture.owner);

    let (status, body) = post(&app, &fixture.uri("stop")).await;
    let still = status_of(&pool, &fixture).await;
    forget(&pool, &fixture).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(still, "pending");
}

#[tokio::test]
async fn somebody_who_is_not_in_the_organisation_may_neither_stop_nor_start() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    let app = app(pool.clone(), fixture.stranger);

    let (stop, stop_body) = post(&app, &fixture.uri("stop")).await;
    let (start, start_body) = post(&app, &fixture.uri("start")).await;
    let still = status_of(&pool, &fixture).await;
    let actions = recorded(&pool, &fixture).await;
    forget(&pool, &fixture).await;

    assert_eq!(stop, StatusCode::FORBIDDEN, "{stop_body}");
    assert_eq!(start, StatusCode::FORBIDDEN, "{start_body}");
    assert_eq!(still, "successful");
    assert!(actions.is_empty());
}

#[tokio::test]
async fn starting_waits_while_billing_has_the_organisation_suspended_and_stopping_does_not() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    suspend_billing(&pool, &fixture).await;
    let app = app(pool.clone(), fixture.owner);

    let (stopped, _) = post(&app, &fixture.uri("stop")).await;
    deliver(&pool, &fixture).await;
    let (refused, refusal) = post(&app, &fixture.uri("start")).await;
    let while_suspended = status_of(&pool, &fixture).await;
    lift_billing_suspension(&pool, &fixture).await;
    let (started, started_body) = post(&app, &fixture.uri("start")).await;
    forget(&pool, &fixture).await;

    assert_eq!(stopped, StatusCode::OK);
    assert_eq!(refused, StatusCode::CONFLICT, "{refusal}");
    assert!(
        refusal["message"]
            .as_str()
            .is_some_and(|message| message.contains("billing")),
        "{refusal}"
    );
    assert_eq!(while_suspended, "stopped");
    assert_eq!(started, StatusCode::OK, "{started_body}");
}

#[tokio::test]
async fn a_second_order_is_refused_until_the_first_has_reached_the_data_plane() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    let app = app(pool.clone(), fixture.owner);

    let (stop, _) = post(&app, &fixture.uri("stop")).await;
    let (start, refusal) = post(&app, &fixture.uri("start")).await;
    let held = status_of(&pool, &fixture).await;
    let held_actions = recorded(&pool, &fixture).await;
    deliver(&pool, &fixture).await;
    let (allowed, allowed_body) = post(&app, &fixture.uri("start")).await;
    forget(&pool, &fixture).await;

    assert_eq!(stop, StatusCode::OK);
    assert_eq!(start, StatusCode::CONFLICT, "{refusal}");
    assert!(
        refusal["message"]
            .as_str()
            .is_some_and(|message| message.contains("has not reached its data plane")),
        "{refusal}"
    );
    assert_eq!(held, "stopped", "a refused order changes nothing");
    assert_eq!(held_actions.len(), 1, "a refused order records nothing");
    assert_eq!(allowed, StatusCode::OK, "{allowed_body}");
}

#[tokio::test]
async fn clicking_stop_and_start_in_a_row_takes_the_first_order_and_refuses_the_rest() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    let app = app(pool.clone(), fixture.owner);

    let mut answers = Vec::new();
    for verb in ["stop", "start", "stop", "start", "stop", "start"] {
        answers.push(post(&app, &fixture.uri(verb)).await.0);
    }
    let actions = recorded(&pool, &fixture).await;
    let status = status_of(&pool, &fixture).await;
    forget(&pool, &fixture).await;

    assert_eq!(
        answers,
        [
            StatusCode::OK,
            StatusCode::CONFLICT,
            StatusCode::CONFLICT,
            StatusCode::CONFLICT,
            StatusCode::CONFLICT,
            StatusCode::CONFLICT
        ]
    );
    assert_eq!(
        actions
            .iter()
            .map(|(kind, _, _)| kind.as_str())
            .collect::<Vec<_>>(),
        ["deployment.stop"]
    );
    assert_eq!(status, "stopped");
}

async fn settle(pool: &PgPool, fixture: &Fixture) {
    sqlx::query("UPDATE deployments SET status = 'successful' WHERE id = $1")
        .bind(fixture.deployment)
        .execute(pool)
        .await
        .expect("a settled deployment");
}

#[tokio::test]
async fn a_stop_waits_until_the_deployment_has_finished_coming_up() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "successful").await;
    let app = app(pool.clone(), fixture.owner);

    let (_, _) = post(&app, &fixture.uri("stop")).await;
    deliver(&pool, &fixture).await;
    let (_, _) = post(&app, &fixture.uri("start")).await;
    deliver(&pool, &fixture).await;
    let (while_coming_up, refusal) = post(&app, &fixture.uri("stop")).await;
    let still = status_of(&pool, &fixture).await;
    settle(&pool, &fixture).await;
    let (once_settled, settled_body) = post(&app, &fixture.uri("stop")).await;
    forget(&pool, &fixture).await;

    assert_eq!(while_coming_up, StatusCode::CONFLICT, "{refusal}");
    assert!(
        refusal["message"]
            .as_str()
            .is_some_and(|message| message.contains("not finished coming up")),
        "{refusal}"
    );
    assert_eq!(still, "in_progress");
    assert_eq!(once_settled, StatusCode::OK, "{settled_body}");
}

#[tokio::test]
async fn a_deployment_that_is_still_being_created_cannot_be_stopped_either() {
    let Some(pool) = pool().await else { return };
    let fixture = Fixture::new();
    seed(&pool, &fixture, "in_progress").await;
    let app = app(pool.clone(), fixture.owner);

    let (status, body) = post(&app, &fixture.uri("stop")).await;
    let actions = recorded(&pool, &fixture).await;
    forget(&pool, &fixture).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(actions.is_empty());
}
