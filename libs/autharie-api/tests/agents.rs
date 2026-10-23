use std::{future::Future, sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    auth_middleware,
    handlers::{
        deployments::agents::{
            create_agent_policy_handler, delete_agent_policy_handler, get_agents_handler,
            list_agent_decisions_handler, list_agent_policies_handler, put_agents_handler,
            update_agent_policy_handler,
        },
        organisations::organisation_settings::{
            get_organisation_handler, update_organisation_handler,
        },
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
use httpmock::{Method::POST, MockServer};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const PERMIT: &str = "permit(principal, action, resource);";

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
        .typed_get(get_agents_handler)
        .typed_put(put_agents_handler)
        .typed_get(list_agent_policies_handler)
        .typed_post(create_agent_policy_handler)
        .typed_put(update_agent_policy_handler)
        .typed_delete(delete_agent_policy_handler)
        .typed_get(list_agent_decisions_handler)
        .typed_get(get_organisation_handler)
        .typed_patch(update_organisation_handler)
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

fn policy(name: &str, enabled: bool) -> Value {
    json!({
        "name": name,
        "description": "why",
        "enabled": enabled,
        "rule": { "effect": "permit", "who": { "any": true } },
        "cedar": PERMIT,
    })
}

#[tokio::test]
async fn an_owner_enables_agents_and_the_data_plane_is_asked_to_apply_them() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);

    let (before_status, before) = call(&app, Method::GET, &estate.agents(), None).await;
    let (put_status, put) = call(
        &app,
        Method::PUT,
        &estate.agents(),
        Some(json!({ "enabled": true, "realm": "acme" })),
    )
    .await;
    let (_, after) = call(&app, Method::GET, &estate.agents(), None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(before_status, StatusCode::OK);
    assert_eq!(before["data"]["enabled"], false);
    assert_eq!(before["data"]["organisation_mcp_enabled"], true);
    assert_eq!(put_status, StatusCode::OK);
    assert_eq!(put["data"]["realm"], "acme");
    assert_eq!(after["data"]["enabled"], true);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0],
        json!({
            "deployment_id": estate.deployment,
            "namespace": estate.deployment.to_string(),
            "enabled": true,
            "realm": "acme",
            "revision": 1,
            "policies": [],
            "access": { "approval_required": false, "approved": [], "blocked": [] },
            "autonomous": [],
            "deleted_agents": [],
            "servers": [],
        })
    );
}

#[tokio::test]
async fn agents_are_sent_off_while_the_organisation_switch_is_off() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, false).await;
    let app = app(&pool, &estate.owner, false, None);

    let (_, put) = call(
        &app,
        Method::PUT,
        &estate.agents(),
        Some(json!({ "enabled": true })),
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(put["data"]["enabled"], true);
    assert_eq!(put["data"]["organisation_mcp_enabled"], false);
    assert_eq!(put["data"]["realm"], "master");
    assert_eq!(sent[0]["enabled"], false);
}

#[tokio::test]
async fn policies_are_created_changed_and_deleted_with_a_revision_each_time() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    let policies = format!("{}/policies", estate.agents());

    let (created_status, created) = call(
        &app,
        Method::POST,
        &policies,
        Some(policy("read-only", true)),
    )
    .await;
    let id = created["data"]["id"].as_str().expect("an id").to_string();
    let (_, off) = call(&app, Method::POST, &policies, Some(policy("off", false))).await;
    let (updated_status, updated) = call(
        &app,
        Method::PUT,
        &format!("{policies}/{id}"),
        Some(policy("read-only-2", true)),
    )
    .await;
    let (_, listed) = call(&app, Method::GET, &policies, None).await;
    let (deleted_status, _) = call(
        &app,
        Method::DELETE,
        &format!("{policies}/{}", off["data"]["id"].as_str().unwrap()),
        None,
    )
    .await;
    let (gone_status, _) = call(
        &app,
        Method::DELETE,
        &format!("{policies}/{}", off["data"]["id"].as_str().unwrap()),
        None,
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(created_status, StatusCode::CREATED);
    assert_eq!(created["data"]["phase"], "Pending");
    assert_eq!(created["data"]["rule"]["effect"], "permit");
    assert_eq!(off["data"]["phase"], Value::Null);
    assert_eq!(updated_status, StatusCode::OK);
    assert_eq!(updated["data"]["name"], "read-only-2");
    assert_eq!(listed["data"].as_array().unwrap().len(), 2);
    assert_eq!(deleted_status, StatusCode::NO_CONTENT);
    assert_eq!(gone_status, StatusCode::NOT_FOUND);

    let revisions: Vec<u64> = sent
        .iter()
        .map(|payload| payload["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, vec![1, 2, 3, 4]);
    assert_eq!(
        sent[3]["policies"],
        json!([{ "id": id, "name": "read-only-2", "cedar": PERMIT }])
    );
}

#[tokio::test]
async fn cedar_that_does_not_parse_is_a_422_and_nothing_is_stored_or_sent() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);
    let policies = format!("{}/policies", estate.agents());

    let mut bad = policy("bad", true);
    bad["cedar"] = json!("permit(principal,");
    let (status, body) = call(&app, Method::POST, &policies, Some(bad)).await;
    let (_, listed) = call(&app, Method::GET, &policies, None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("the Cedar policy does not parse")
    );
    assert!(listed["data"].as_array().unwrap().is_empty());
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_member_without_the_right_to_manage_instances_changes_nothing() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.member, false, None);

    let (put, _) = call(
        &app,
        Method::PUT,
        &estate.agents(),
        Some(json!({ "enabled": true })),
    )
    .await;
    let (post, _) = call(
        &app,
        Method::POST,
        &format!("{}/policies", estate.agents()),
        Some(policy("p", true)),
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(put, StatusCode::FORBIDDEN);
    assert_eq!(post, StatusCode::FORBIDDEN);
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_keycloak_deployment_cannot_have_agents() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);

    let (status, _) = call(
        &app,
        Method::PUT,
        &format!(
            "/organisations/{}/deployments/{}/agents",
            estate.organisation, estate.keycloak
        ),
        Some(json!({ "enabled": true })),
    )
    .await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_agent_cannot_edit_its_own_policies_or_read_the_agent_routes() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let owner = app(&pool, &estate.owner, false, None);
    let agent = app(&pool, &estate.owner, true, None);
    let policies = format!("{}/policies", estate.agents());
    let (_, created) = call(&owner, Method::POST, &policies, Some(policy("p", true))).await;
    let id = created["data"]["id"].as_str().unwrap().to_string();

    let attempts = [
        (Method::GET, estate.agents(), None),
        (
            Method::PUT,
            estate.agents(),
            Some(json!({ "enabled": false })),
        ),
        (Method::GET, policies.clone(), None),
        (Method::POST, policies.clone(), Some(policy("evil", true))),
        (
            Method::PUT,
            format!("{policies}/{id}"),
            Some(policy("evil", true)),
        ),
        (Method::DELETE, format!("{policies}/{id}"), None),
        (Method::GET, format!("{}/decisions", estate.agents()), None),
    ];
    let mut refused = Vec::new();
    for (method, uri, body) in attempts {
        let (status, answer) = call(&agent, method, &uri, body).await;
        refused.push((status, answer["code"].clone()));
    }
    let (_, listed) = call(&owner, Method::GET, &policies, None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    for (status, code) in refused {
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(code, "E_DELEGATED_ROUTE_FORBIDDEN");
    }
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed["data"][0]["name"], "p");
    assert_eq!(sent.len(), 1);
}

#[tokio::test]
async fn turning_the_organisation_switch_resends_the_deployments_with_agent_settings() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, false).await;
    let app = app(&pool, &estate.owner, false, None);
    let organisation = format!("/organisations/{}", estate.organisation);
    call(
        &app,
        Method::PUT,
        &estate.agents(),
        Some(json!({ "enabled": true })),
    )
    .await;
    call(
        &app,
        Method::POST,
        &format!("{}/policies", estate.agents()),
        Some(policy("p", true)),
    )
    .await;

    let (on_status, _) = call(
        &app,
        Method::PATCH,
        &organisation,
        Some(json!({ "mcp_enabled": true })),
    )
    .await;
    let (same_status, _) = call(
        &app,
        Method::PATCH,
        &organisation,
        Some(json!({ "mcp_enabled": true })),
    )
    .await;
    let (off_status, _) = call(
        &app,
        Method::PATCH,
        &organisation,
        Some(json!({ "mcp_enabled": false })),
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(
        (on_status, same_status, off_status),
        (StatusCode::OK, StatusCode::OK, StatusCode::OK)
    );
    let flags: Vec<(u64, bool)> = sent
        .iter()
        .map(|payload| {
            (
                payload["revision"].as_u64().unwrap(),
                payload["enabled"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(flags, vec![(1, false), (2, false), (3, true), (4, false)]);
    assert_eq!(sent[3]["policies"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_decisions_answer_409_when_no_index_is_configured() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false, None);

    let (status, _) = call(
        &app,
        Method::GET,
        &format!("{}/decisions", estate.agents()),
        None,
    )
    .await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn the_decisions_are_searched_in_the_organisations_index_for_this_deployment() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let quickwit = MockServer::start();
    let search = quickwit.mock(|when, then| {
        when.method(POST)
            .path(format!("/api/v1/logs-{}/search", estate.organisation))
            .body_contains("kind:agent_decision")
            .body_contains(estate.deployment.to_string())
            .body_contains("decision:\\\"deny\\\"")
            .body_contains("tool:\\\"list_users\\\"")
            .body_contains("subject_id:\\\"alice\\\"");
        then.status(200).json_body(json!({
            "hits": [
                {
                    "timestamp": "2026-10-22T10:00:00Z",
                    "kind": "agent_decision",
                    "deployment_id": estate.deployment,
                    "server": "ferriskey",
                    "method": "tools/call",
                    "tool": "list_users",
                    "subject_type": "User",
                    "subject_id": "alice",
                    "agent": "claude",
                    "delegated": true,
                    "decision": "deny",
                    "policies": ["p1"],
                    "reason": "no_permit",
                    "request_id": "r1"
                },
                { "timestamp": "2026-10-22T09:00:00Z", "decision": "not-a-verdict" }
            ]
        }));
    });
    let app = app(
        &pool,
        &estate.owner,
        false,
        Some(Arc::new(QuickwitLogSearchIndex::new(quickwit.base_url()))),
    );

    let (status, body) = call(
        &app,
        Method::GET,
        &format!(
            "{}/decisions?decision=deny&tool=list_users&subject=alice&limit=10&from=2026-10-22T00:00:00Z&to=2026-10-23T00:00:00Z",
            estate.agents()
        ),
        None,
    )
    .await;
    let (bad, _) = call(
        &app,
        Method::GET,
        &format!("{}/decisions?decision=maybe", estate.agents()),
        None,
    )
    .await;
    estate.forget(&pool).await;

    search.assert();
    assert_eq!(status, StatusCode::OK);
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["decision"], "deny");
    assert_eq!(data[0]["tool"], "list_users");
    assert_eq!(data[0]["policies"], json!(["p1"]));
    assert_eq!(data[0]["time"], "2026-10-22T10:00:00Z");
    assert_eq!(data[0]["deployment_id"], estate.deployment.to_string());
    assert_eq!(bad, StatusCode::BAD_REQUEST);
}
