use std::{future::Future, sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    auth_middleware,
    handlers::{
        organisations::organisation_settings::{
            get_organisation_handler, update_organisation_handler,
        },
        users::get_user_organisations::get_user_organisations_handler,
    },
    state::AppState,
};
use autharie_auth::{Actor, Identity, User};
use autharie_core::{
    AutharieService, CoreError,
    auth::ports::AuthService,
    organisation::{OrganisationId, ports::OrganisationMcpSwitch},
    user::{commands::CreateUserCommand, ports::UserService},
};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
    middleware::from_fn_with_state,
    routing::get,
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

#[derive(Clone)]
struct Gate {
    service: AutharieService,
    identity: Identity,
}

impl AuthService for Gate {
    fn get_identity(
        &self,
        _token: &str,
    ) -> impl Future<Output = Result<Identity, CoreError>> + Send {
        let identity = self.identity.clone();
        async move { Ok(identity) }
    }
}

impl UserService for Gate {
    fn create_user(
        &self,
        command: CreateUserCommand,
    ) -> impl Future<Output = Result<autharie_core::user::User, CoreError>> + Send {
        self.service.create_user(command)
    }
}

impl OrganisationMcpSwitch for Gate {
    fn mcp_enabled(
        &self,
        id: OrganisationId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send {
        self.service.mcp_enabled(id)
    }
}

struct Person {
    id: Uuid,
}

impl Person {
    async fn seed(pool: &PgPool) -> Self {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'mcp-switch', $3)")
            .bind(id)
            .bind(format!("{id}@mcp-switch-api.test"))
            .bind(id.to_string())
            .execute(pool)
            .await
            .expect("a user");
        Self { id }
    }

    fn identity(&self, delegated: bool) -> Identity {
        Identity::User(User {
            scopes: Vec::new(),
            act: delegated.then(|| Actor {
                sub: Some("gateway".to_string()),
                client_id: Some("mcp-gateway".to_string()),
            }),
            id: self.id.to_string(),
            username: "mcp-switch".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })
    }
}

async fn organisation(pool: &PgPool, owner: &Person, mcp_enabled: bool) -> Uuid {
    let organisation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, mcp_enabled, created_at, updated_at) \
         VALUES ($1, 'mcp-switch', $2, $3, 'active', 'free', 5, 5, 5, $4, now(), now())",
    )
    .bind(organisation)
    .bind(format!("mcp-switch-{organisation}"))
    .bind(owner.id)
    .bind(mcp_enabled)
    .execute(pool)
    .await
    .expect("an organisation");
    sqlx::query(
        "INSERT INTO members (id, organisation_id, user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(organisation)
    .bind(owner.id)
    .execute(pool)
    .await
    .expect("a membership");
    organisation
}

async fn join(pool: &PgPool, organisation: Uuid, person: &Person) {
    sqlx::query(
        "INSERT INTO members (id, organisation_id, user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(organisation)
    .bind(person.id)
    .execute(pool)
    .await
    .expect("a membership");
}

async fn forget(pool: &PgPool, organisations: &[Uuid], people: &[&Person]) {
    for organisation in organisations {
        sqlx::query("DELETE FROM members WHERE organisation_id = $1")
            .bind(organisation)
            .execute(pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(organisation)
            .execute(pool)
            .await
            .expect("cleanup");
    }
    for person in people {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(person.id)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

fn app(pool: &PgPool, person: &Person, delegated: bool) -> Router {
    let service = AutharieService::new(pool.clone());
    let gate = Gate {
        service: service.clone(),
        identity: person.identity(delegated),
    };

    Router::new()
        .typed_get(get_organisation_handler)
        .typed_patch(update_organisation_handler)
        .typed_get(get_user_organisations_handler)
        .route(
            "/organisations/{organisation_id}/deployments",
            get(|| async { "deployments" }),
        )
        .layer(from_fn_with_state(gate, auth_middleware::<Gate>))
        .with_state(AppState {
            args: Arc::new(Args::default()),
            service,
            certificate_source: None,
            quickwit_search: None,
            quickwit_traces: None,
            billing: None,
        })
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", "Bearer token")
        .header("content-type", "application/json")
        .body(match body {
            Some(body) => Body::from(body.to_string()),
            None => Body::empty(),
        })
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
async fn a_delegated_call_to_an_organisation_with_the_switch_off_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool).await;
    let off = organisation(&pool, &owner, false).await;
    let on = organisation(&pool, &owner, true).await;

    let uri = |organisation: Uuid| format!("/organisations/{organisation}/deployments");
    let refused = call(&app(&pool, &owner, true), Method::GET, &uri(off), None).await;
    let allowed = call(&app(&pool, &owner, true), Method::GET, &uri(on), None).await;
    let ordinary = call(&app(&pool, &owner, false), Method::GET, &uri(off), None).await;

    forget(&pool, &[off, on], &[&owner]).await;

    assert_eq!(refused.0, StatusCode::FORBIDDEN);
    assert_eq!(refused.1["code"], "E_ORGANISATION_MCP_DISABLED");
    assert_eq!(
        refused.1["message"],
        "AI access is turned off for this organisation"
    );
    assert_eq!(allowed.0, StatusCode::OK);
    assert_eq!(ordinary.0, StatusCode::OK);
}

#[tokio::test]
async fn a_delegated_call_to_an_organisation_that_does_not_exist_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool).await;

    let (status, body) = call(
        &app(&pool, &owner, true),
        Method::GET,
        &format!("/organisations/{}/deployments", Uuid::new_v4()),
        None,
    )
    .await;

    forget(&pool, &[], &[&owner]).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "E_ORGANISATION_MCP_DISABLED");
}

#[tokio::test]
async fn the_organisation_list_leaves_out_what_the_switch_closes_for_an_agent() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool).await;
    let off = organisation(&pool, &owner, false).await;
    let on = organisation(&pool, &owner, true).await;

    let ids = |body: &Value| -> Vec<String> {
        body["data"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|organisation| organisation["id"].as_str().expect("an id").to_string())
            .collect()
    };
    let (_, as_agent) = call(
        &app(&pool, &owner, true),
        Method::GET,
        "/users/@me/organisations",
        None,
    )
    .await;
    let (_, as_user) = call(
        &app(&pool, &owner, false),
        Method::GET,
        "/users/@me/organisations",
        None,
    )
    .await;

    forget(&pool, &[off, on], &[&owner]).await;

    let (agent, user) = (ids(&as_agent), ids(&as_user));
    assert_eq!(agent, vec![on.to_string()]);
    assert!(user.contains(&on.to_string()) && user.contains(&off.to_string()));
}

#[tokio::test]
async fn an_owner_turns_the_switch_on_and_reads_it_back() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool).await;
    let organisation = organisation(&pool, &owner, false).await;
    let uri = format!("/organisations/{organisation}");

    let (before_status, before) = call(&app(&pool, &owner, false), Method::GET, &uri, None).await;
    let (patched_status, patched) = call(
        &app(&pool, &owner, false),
        Method::PATCH,
        &uri,
        Some(json!({ "mcp_enabled": true })),
    )
    .await;
    let (_, after) = call(&app(&pool, &owner, false), Method::GET, &uri, None).await;

    forget(&pool, &[organisation], &[&owner]).await;

    assert_eq!(before_status, StatusCode::OK);
    assert_eq!(before["data"]["mcp_enabled"], false);
    assert_eq!(patched_status, StatusCode::OK);
    assert_eq!(patched["data"]["mcp_enabled"], true);
    assert_eq!(after["data"]["mcp_enabled"], true);
}

#[tokio::test]
async fn a_member_who_cannot_manage_the_organisation_cannot_turn_the_switch_on() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool).await;
    let member = Person::seed(&pool).await;
    let organisation = organisation(&pool, &owner, false).await;
    join(&pool, organisation, &member).await;
    let uri = format!("/organisations/{organisation}");

    let (status, _) = call(
        &app(&pool, &member, false),
        Method::PATCH,
        &uri,
        Some(json!({ "mcp_enabled": true })),
    )
    .await;
    let (_, after) = call(&app(&pool, &owner, false), Method::GET, &uri, None).await;

    forget(&pool, &[organisation], &[&owner, &member]).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(after["data"]["mcp_enabled"], false);
}

#[tokio::test]
async fn a_delegated_token_cannot_patch_or_read_the_organisation_itself() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool).await;
    let organisation = organisation(&pool, &owner, true).await;
    let uri = format!("/organisations/{organisation}");

    let (patch_status, patch) = call(
        &app(&pool, &owner, true),
        Method::PATCH,
        &uri,
        Some(json!({ "mcp_enabled": false })),
    )
    .await;
    let (get_status, get) = call(&app(&pool, &owner, true), Method::GET, &uri, None).await;
    let (_, after) = call(&app(&pool, &owner, false), Method::GET, &uri, None).await;

    forget(&pool, &[organisation], &[&owner]).await;

    assert_eq!(patch_status, StatusCode::FORBIDDEN);
    assert_eq!(patch["code"], "E_DELEGATED_ROUTE_FORBIDDEN");
    assert_eq!(get_status, StatusCode::FORBIDDEN);
    assert_eq!(get["code"], "E_DELEGATED_ROUTE_FORBIDDEN");
    assert_eq!(after["data"]["mcp_enabled"], true);
}
