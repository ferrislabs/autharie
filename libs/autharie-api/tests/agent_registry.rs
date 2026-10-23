use std::{future::Future, sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    auth_middleware,
    handlers::deployments::{
        agent_registry::{
            approve_agent_handler, block_agent_handler, create_autonomous_agent_handler,
            delete_agent_handler, get_agent_credential_handler, list_agent_clients_handler,
            put_agent_access_handler, rotate_autonomous_secret_handler, unblock_agent_handler,
        },
        agents::put_agents_handler,
    },
    quickwit::logs::QuickwitLogSearchIndex,
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
};
use axum_extra::routing::RouterExt;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::{Method::POST, MockServer};
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
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'agents', $3)")
            .bind(id)
            .bind(format!("{id}@agents-api.test"))
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
            username: "agents".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })
    }
}

struct Estate {
    owner: Person,
    member: Person,
    organisation: Uuid,
    data_plane: Uuid,
    deployment: Uuid,
    keycloak: Uuid,
}

impl Estate {
    async fn seed(pool: &PgPool, mcp_enabled: bool) -> Self {
        let owner = Person::seed(pool).await;
        let member = Person::seed(pool).await;
        let estate = Self {
            owner,
            member,
            organisation: Uuid::new_v4(),
            data_plane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            keycloak: Uuid::new_v4(),
        };

        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, mcp_enabled, created_at, updated_at) \
             VALUES ($1, 'agents', $1::text, $2, 'active', 'free', 5, 5, 5, $3, now(), now())",
        )
        .bind(estate.organisation)
        .bind(estate.owner.id)
        .bind(mcp_enabled)
        .execute(pool)
        .await
        .expect("an organisation");
        for person in [&estate.owner, &estate.member] {
            sqlx::query(
                "INSERT INTO members (id, organisation_id, user_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, now(), now())",
            )
            .bind(Uuid::new_v4())
            .bind(estate.organisation)
            .bind(person.id)
            .execute(pool)
            .await
            .expect("a membership");
        }
        sqlx::query(
            "INSERT INTO data_planes \
             (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
              capacity_storage_gib, created_at, updated_at) \
             VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200, now(), now())",
        )
        .bind(estate.data_plane)
        .bind(format!("agents-{}", estate.data_plane))
        .execute(pool)
        .await
        .expect("a data plane");
        for (deployment, kind) in [
            (estate.deployment, "ferriskey"),
            (estate.keycloak, "keycloak"),
        ] {
            sqlx::query(
                "INSERT INTO deployments \
                 (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
                  cpu_millis, memory_mib, storage_gib, created_by, created_at, \
                  updated_at, hostname_slug) \
                 VALUES ($1, $2, $3, 'auth', $4, 'successful', $1::text, '26.0.0', \
                         500, 1024, 1, $5, now(), now(), $1::text)",
            )
            .bind(deployment)
            .bind(estate.organisation)
            .bind(estate.data_plane)
            .bind(kind)
            .bind(estate.owner.id)
            .execute(pool)
            .await
            .expect("a deployment");
        }

        estate
    }

    fn agents(&self) -> String {
        format!(
            "/organisations/{}/deployments/{}/agents",
            self.organisation, self.deployment
        )
    }

    async fn sent(&self, pool: &PgPool) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT payload FROM actions \
             WHERE deployment_id = $1 AND action_type = 'deployment.agents' \
             ORDER BY created_at, id",
        )
        .bind(self.deployment)
        .fetch_all(pool)
        .await
        .expect("actions")
    }

    async fn forget(&self, pool: &PgPool) {
        for (statement, id) in [
            (
                "DELETE FROM actions WHERE deployment_id = $1",
                self.deployment,
            ),
            ("DELETE FROM deployments WHERE id = $1", self.deployment),
            ("DELETE FROM deployments WHERE id = $1", self.keycloak),
            ("DELETE FROM data_planes WHERE id = $1", self.data_plane),
            (
                "DELETE FROM members WHERE organisation_id = $1",
                self.organisation,
            ),
            ("DELETE FROM organisations WHERE id = $1", self.organisation),
            ("DELETE FROM users WHERE id = $1", self.owner.id),
            ("DELETE FROM users WHERE id = $1", self.member.id),
        ] {
            sqlx::query(statement)
                .bind(id)
                .execute(pool)
                .await
                .expect("cleanup");
        }
    }
}

fn app(
    pool: &PgPool,
    person: &Person,
    delegated: bool,
    quickwit: Option<Arc<QuickwitLogSearchIndex>>,
) -> Router {
    let service = AutharieService::new(pool.clone());
    let gate = Gate {
        service: service.clone(),
        identity: person.identity(delegated),
    };

    Router::new()
        .typed_put(put_agents_handler)
        .typed_get(list_agent_clients_handler)
        .typed_put(put_agent_access_handler)
        .typed_post(approve_agent_handler)
        .typed_post(block_agent_handler)
        .typed_post(unblock_agent_handler)
        .typed_delete(delete_agent_handler)
        .typed_post(create_autonomous_agent_handler)
        .typed_post(rotate_autonomous_secret_handler)
        .typed_get(get_agent_credential_handler)
        .layer(from_fn_with_state(gate, auth_middleware::<Gate>))
        .with_state(AppState {
            args: Arc::new(Args::default()),
            service,
            certificate_source: None,
            quickwit_search: quickwit,
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

fn jwk(bytes: usize) -> Value {
    let mut modulus = vec![0xffu8; bytes];
    modulus[bytes - 1] = 0xfd;
    json!({
        "kty": "RSA",
        "alg": "RSA-OAEP-256",
        "e": "AQAB",
        "n": URL_SAFE_NO_PAD.encode(modulus),
    })
}

async fn enable_agents(app: &Router, estate: &Estate) {
    let (status, _) = call(
        app,
        Method::PUT,
        &estate.agents(),
        Some(json!({ "enabled": true, "realm": "acme" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

fn clients(estate: &Estate) -> String {
    format!("{}/clients", estate.agents())
}

async fn report(pool: &PgPool, estate: &Estate, id: &str, source: &str) {
    sqlx::query(
        "INSERT INTO agent_clients \
         (deployment_id, client_id, name, source, kind, enabled, created_at, reported_at) \
         VALUES ($1, $2, $2, $3, 'delegated', true, now(), now())",
    )
    .bind(estate.deployment)
    .bind(id)
    .bind(source)
    .execute(pool)
    .await
    .expect("a reported client");
}

#[tokio::test]
async fn spec_ar_1_the_registry_lists_the_reported_dcr_and_cimd_clients_with_source_and_kind() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    report(&pool, &estate, "dcr-client", "dynamic").await;
    report(
        &pool,
        &estate,
        "https://cimd.example/client.json",
        "metadata_document",
    )
    .await;
    let app = app(&pool, &estate.owner, false, None);

    let (status, body) = call(&app, Method::GET, &clients(&estate), None).await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::OK);
    let found: Vec<(&str, &str, &str)> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["client_id"].as_str().unwrap(),
                c["source"].as_str().unwrap(),
                c["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(found.contains(&("dcr-client", "dynamic", "delegated")));
    assert!(found.contains(&(
        "https://cimd.example/client.json",
        "metadata_document",
        "delegated"
    )));
    assert_eq!(found.len(), 2);
    assert_eq!(body["truncated"], false);
    assert_eq!(body["approval_required"], false);
}

#[tokio::test]
async fn spec_ar_7_the_registry_carries_the_newest_decision_time_of_each_agent() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    report(&pool, &estate, "dcr-client", "dynamic").await;
    let quickwit = MockServer::start();
    let aggregation = quickwit.mock(|when, then| {
        when.method(POST)
            .path(format!("/api/v1/logs-{}/search", estate.organisation))
            .body_contains("kind:agent_decision")
            .body_contains(estate.deployment.to_string())
            .body_contains("\"by_agent\"");
        then.status(200).json_body(json!({
            "num_hits": 0,
            "hits": [],
            "aggregations": { "by_agent": { "buckets": [
                { "key": "dcr-client", "doc_count": 4, "last": { "value": 1_800_000_000_000.0 } },
                { "key": "stranger", "doc_count": 1, "last": { "value": 1_700_000_000_000.0 } }
            ] } }
        }));
    });
    let app = app(
        &pool,
        &estate.owner,
        false,
        Some(Arc::new(QuickwitLogSearchIndex::new(quickwit.base_url()))),
    );

    let (status, body) = call(&app, Method::GET, &clients(&estate), None).await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::OK);
    aggregation.assert();
    let by_id = |id: &str| {
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["client_id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(by_id("dcr-client")["last_seen_at"], "2027-01-15T08:00:00Z");
    assert_eq!(by_id("stranger")["source"], "seen");
}

#[tokio::test]
async fn spec_ar_2_approval_mode_and_approval_reach_the_data_plane_and_the_registry() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    report(&pool, &estate, "dcr-client", "dynamic").await;
    let app = app(&pool, &estate.owner, false, None);
    enable_agents(&app, &estate).await;

    let (mode, _) = call(
        &app,
        Method::PUT,
        &format!("{}/access", estate.agents()),
        Some(json!({ "approval_required": true })),
    )
    .await;
    let (_, pending) = call(&app, Method::GET, &clients(&estate), None).await;
    let (approved, _) = call(
        &app,
        Method::POST,
        &format!("{}/dcr-client/approve", clients(&estate)),
        None,
    )
    .await;
    let (_, after) = call(&app, Method::GET, &clients(&estate), None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(mode, StatusCode::OK);
    assert_eq!(pending["data"][0]["access"], "pending");
    assert_eq!(pending["approval_required"], true);
    assert_eq!(approved, StatusCode::NO_CONTENT);
    assert_eq!(after["data"][0]["access"], "approved");
    let last = sent.last().unwrap();
    assert_eq!(last["access"]["approval_required"], true);
    assert_eq!(last["access"]["approved"], json!(["dcr-client"]));
}

#[tokio::test]
async fn spec_ar_3_blocking_an_agent_and_unblocking_it_reach_the_data_plane() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    let block = format!("{}/dcr-client/block", clients(&estate));
    let unblock = format!("{}/dcr-client/unblock", clients(&estate));

    let (blocked, _) = call(&app, Method::POST, &block, None).await;
    let (unblocked, _) = call(&app, Method::POST, &unblock, None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(blocked, StatusCode::NO_CONTENT);
    assert_eq!(unblocked, StatusCode::NO_CONTENT);
    assert_eq!(sent[0]["access"]["blocked"], json!(["dcr-client"]));
    assert_eq!(sent[1]["access"]["blocked"], json!([]));
}

#[tokio::test]
async fn deleting_an_agent_asks_the_data_plane_to_delete_it() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("{}/dcr-client", clients(&estate)),
        None,
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(sent[0]["deleted_agents"], json!(["dcr-client"]));
}

#[tokio::test]
async fn spec_ar_5_the_sealed_secret_is_returned_once_then_410_and_nothing_is_left_stored() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    enable_agents(&app, &estate).await;

    let (created, body) = call(
        &app,
        Method::POST,
        &format!("{}/autonomous", estate.agents()),
        Some(json!({ "name": "nightly sync", "sealing_key": jwk(256) })),
    )
    .await;
    let request_id = body["data"]["request_id"].as_str().unwrap().to_string();
    let credential = format!("{}/credentials/{request_id}", estate.agents());
    let (waiting, waiting_body) = call(&app, Method::GET, &credential, None).await;
    let client_id: String =
        sqlx::query_scalar("SELECT client_id FROM agent_credentials WHERE request_id = $1::uuid")
            .bind(&request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE deployment_agents SET mcp_url = 'https://auth.acme.test/mcp' WHERE deployment_id = $1")
        .bind(estate.deployment)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_credentials SET sealed = 'c2VhbGVk' WHERE request_id = $1::uuid")
        .bind(&request_id)
        .execute(&pool)
        .await
        .unwrap();
    let (first, first_body) = call(&app, Method::GET, &credential, None).await;
    let (second, _) = call(&app, Method::GET, &credential, None).await;
    let left: Option<String> =
        sqlx::query_scalar("SELECT sealed FROM agent_credentials WHERE request_id = $1::uuid")
            .bind(&request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(created, StatusCode::ACCEPTED);
    assert!(client_id.starts_with("agent-nightly-sync-"));
    assert_eq!(waiting, StatusCode::ACCEPTED);
    assert_eq!(waiting_body["data"]["status"], "pending");
    assert_eq!(first, StatusCode::OK);
    assert_eq!(first_body["data"]["sealed"], "c2VhbGVk");
    assert_eq!(first_body["data"]["client_id"], client_id);
    assert!(first_body["data"]["token_url"].as_str().is_some());
    assert!(first_body["data"]["audience"].as_str().is_some());
    assert_eq!(second, StatusCode::GONE);
    assert_eq!(left, None);
    assert_eq!(sent[1]["autonomous"][0]["request_id"], request_id);
    assert!(
        sent[1]["autonomous"][0]["sealing_key"]["n"]
            .as_str()
            .is_some()
    );
    assert!(sent[2]["autonomous"][0].get("request_id").is_none());
}

#[tokio::test]
async fn spec_ar_6_a_weak_or_wrong_key_is_a_400_and_nothing_is_created() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    enable_agents(&app, &estate).await;
    let uri = format!("{}/autonomous", estate.agents());

    let (small, _) = call(
        &app,
        Method::POST,
        &uri,
        Some(json!({ "name": "a", "sealing_key": jwk(255) })),
    )
    .await;
    let (ec, _) = call(
        &app,
        Method::POST,
        &uri,
        Some(json!({ "name": "a", "sealing_key": { "kty": "EC", "crv": "P-256", "x": "AA", "y": "AA", "n": "AA", "e": "AQAB" } })),
    )
    .await;
    let (private, _) = call(
        &app,
        Method::POST,
        &uri,
        Some(json!({ "name": "a", "sealing_key": { "d": "AAAA", "kty": "RSA", "alg": "RSA-OAEP-256", "e": "AQAB", "n": jwk(256)["n"] } })),
    )
    .await;
    let stored: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM agent_autonomous WHERE deployment_id = $1) \
              + (SELECT count(*) FROM agent_credentials WHERE deployment_id = $1)",
    )
    .bind(estate.deployment)
    .fetch_one(&pool)
    .await
    .unwrap();
    estate.forget(&pool).await;

    assert_eq!(small, StatusCode::BAD_REQUEST);
    assert_eq!(ec, StatusCode::BAD_REQUEST);
    assert_eq!(private, StatusCode::BAD_REQUEST);
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn spec_ar_6_a_credential_not_fetched_within_fifteen_minutes_is_404_and_deleted() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    enable_agents(&app, &estate).await;
    let (_, body) = call(
        &app,
        Method::POST,
        &format!("{}/autonomous", estate.agents()),
        Some(json!({ "name": "late", "sealing_key": jwk(256) })),
    )
    .await;
    let request_id = body["data"]["request_id"].as_str().unwrap().to_string();
    sqlx::query(
        "UPDATE agent_credentials SET sealed = 'c2VhbGVk', expires_at = now() - interval '1 minute' \
         WHERE request_id = $1::uuid",
    )
    .bind(&request_id)
    .execute(&pool)
    .await
    .unwrap();

    let (status, _) = call(
        &app,
        Method::GET,
        &format!("{}/credentials/{request_id}", estate.agents()),
        None,
    )
    .await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_credentials WHERE request_id = $1::uuid")
            .bind(&request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn an_autonomous_agent_needs_agents_enabled_first() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);

    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{}/autonomous", estate.agents()),
        Some(json!({ "name": "a", "sealing_key": jwk(256) })),
    )
    .await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_member_without_manage_rights_changes_nothing_and_collects_no_credential() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.member, false, None);

    let (approve, _) = call(
        &app,
        Method::POST,
        &format!("{}/a/approve", clients(&estate)),
        None,
    )
    .await;
    let (mode, _) = call(
        &app,
        Method::PUT,
        &format!("{}/access", estate.agents()),
        Some(json!({ "approval_required": true })),
    )
    .await;
    let (credential, _) = call(
        &app,
        Method::GET,
        &format!("{}/credentials/{}", estate.agents(), Uuid::new_v4()),
        None,
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(approve, StatusCode::FORBIDDEN);
    assert_eq!(mode, StatusCode::FORBIDDEN);
    assert_eq!(credential, StatusCode::FORBIDDEN);
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_keycloak_deployment_has_no_registry() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);

    let (status, _) = call(
        &app,
        Method::GET,
        &format!(
            "/organisations/{}/deployments/{}/agents/clients",
            estate.organisation, estate.keycloak
        ),
        None,
    )
    .await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_agent_cannot_reach_any_registry_route() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let agent = app(&pool, &estate.owner, true, None);
    let id = Uuid::new_v4();
    let attempts = [
        (Method::GET, clients(&estate), None),
        (
            Method::PUT,
            format!("{}/access", estate.agents()),
            Some(json!({ "approval_required": true })),
        ),
        (
            Method::POST,
            format!("{}/x/approve", clients(&estate)),
            None,
        ),
        (Method::POST, format!("{}/x/block", clients(&estate)), None),
        (
            Method::POST,
            format!("{}/x/unblock", clients(&estate)),
            None,
        ),
        (Method::DELETE, format!("{}/x", clients(&estate)), None),
        (
            Method::POST,
            format!("{}/autonomous", estate.agents()),
            Some(json!({ "name": "a", "sealing_key": jwk(256) })),
        ),
        (
            Method::POST,
            format!("{}/autonomous/x/rotate", estate.agents()),
            Some(json!({ "sealing_key": jwk(256) })),
        ),
        (
            Method::GET,
            format!("{}/credentials/{id}", estate.agents()),
            None,
        ),
    ];
    let mut refused = Vec::new();
    for (method, uri, body) in attempts {
        let (status, answer) = call(&agent, method, &uri, body).await;
        refused.push((status, answer["code"].clone()));
    }
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    for (status, code) in refused {
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(code, "E_DELEGATED_ROUTE_FORBIDDEN");
    }
    assert!(sent.is_empty());
}

#[tokio::test]
async fn rotating_an_agent_autharie_did_not_create_is_a_404() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    enable_agents(&app, &estate).await;

    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{}/autonomous/dcr-client/rotate", estate.agents()),
        Some(json!({ "sealing_key": jwk(256) })),
    )
    .await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
