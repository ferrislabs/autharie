use std::{future::Future, sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    auth_middleware,
    handlers::deployments::{
        agent_servers::{
            discover_agent_server_handler, list_agent_servers_handler,
            register_agent_server_handler, remove_agent_server_handler,
            update_agent_server_handler,
        },
        agents::{create_agent_policy_handler, list_agent_policies_handler, put_agents_handler},
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

fn app(pool: &PgPool, person: &Person, delegated: bool) -> Router {
    let service = AutharieService::new(pool.clone());
    let gate = Gate {
        service: service.clone(),
        identity: person.identity(delegated),
    };

    Router::new()
        .typed_put(put_agents_handler)
        .typed_get(list_agent_policies_handler)
        .typed_post(create_agent_policy_handler)
        .typed_get(list_agent_servers_handler)
        .typed_post(register_agent_server_handler)
        .typed_put(update_agent_server_handler)
        .typed_delete(remove_agent_server_handler)
        .typed_post(discover_agent_server_handler)
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

fn servers(estate: &Estate) -> String {
    format!("{}/servers", estate.agents())
}

fn crm() -> Value {
    json!({ "slug": "crm", "name": "CRM", "url": "https://tools.example.com/mcp" })
}

#[tokio::test]
async fn a_server_is_registered_listed_changed_and_removed_with_a_revision_each_time() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false);
    call(
        &app,
        Method::PUT,
        &estate.agents(),
        Some(json!({ "enabled": true })),
    )
    .await;

    let (created_status, created) = call(&app, Method::POST, &servers(&estate), Some(crm())).await;
    let id = created["data"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let (_, listed) = call(&app, Method::GET, &servers(&estate), None).await;
    let (changed_status, changed) = call(
        &app,
        Method::PUT,
        &format!("{}/{id}", servers(&estate)),
        Some(
            json!({ "name": "Billing", "url": "https://other.example.com/mcp", "enabled": false }),
        ),
    )
    .await;
    let (discover_status, _) = call(
        &app,
        Method::POST,
        &format!("{}/{id}/discover", servers(&estate)),
        None,
    )
    .await;
    let (deleted_status, _) = call(
        &app,
        Method::DELETE,
        &format!("{}/{id}", servers(&estate)),
        None,
    )
    .await;
    let (_, after) = call(&app, Method::GET, &servers(&estate), None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(created_status, StatusCode::CREATED);
    assert_eq!(created["data"]["slug"], "crm");
    assert_eq!(created["data"]["enabled"], true);
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed["data"][0]["discovery"], Value::Null);
    assert_eq!(changed_status, StatusCode::OK);
    assert_eq!(changed["data"]["name"], "Billing");
    assert_eq!(changed["data"]["slug"], "crm");
    assert_eq!(discover_status, StatusCode::ACCEPTED);
    assert_eq!(deleted_status, StatusCode::NO_CONTENT);
    assert_eq!(after["data"], json!([]));

    let revisions: Vec<u64> = sent
        .iter()
        .map(|p| p["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, [1, 2, 3, 4, 5]);
    assert_eq!(sent[1]["servers"][0]["slug"], "crm");
    assert_eq!(
        sent[1]["servers"][0]["url"],
        "https://tools.example.com/mcp"
    );
    assert_eq!(sent[2]["servers"][0]["enabled"], false);
    assert_eq!(sent[4]["servers"], json!([]));
}

#[tokio::test]
async fn an_address_that_is_not_public_https_or_a_reserved_slug_is_a_400_and_nothing_is_stored() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false);

    let mut answers = Vec::new();
    for body in [
        json!({ "slug": "crm", "name": "CRM", "url": "http://tools.example.com/mcp" }),
        json!({ "slug": "crm", "name": "CRM", "url": "https://10.0.0.5/mcp" }),
        json!({ "slug": "crm", "name": "CRM", "url": "https://user:pw@x.example/" }),
        json!({ "slug": "ferriskey", "name": "CRM", "url": "https://x.example/mcp" }),
    ] {
        answers.push(call(&app, Method::POST, &servers(&estate), Some(body)).await);
    }
    let (_, listed) = call(&app, Method::GET, &servers(&estate), None).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    for (status, answer) in &answers {
        assert_eq!(*status, StatusCode::BAD_REQUEST, "{answer}");
    }
    assert!(answers[0].1["message"].as_str().unwrap().contains("url"));
    assert!(answers[3].1["message"].as_str().unwrap().contains("slug"));
    assert_eq!(listed["data"], json!([]));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_slug_cannot_be_taken_twice_and_a_deployment_stops_at_ten() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false);

    call(&app, Method::POST, &servers(&estate), Some(crm())).await;
    let (twice, _) = call(&app, Method::POST, &servers(&estate), Some(crm())).await;
    for index in 1..10 {
        let (status, _) = call(
            &app,
            Method::POST,
            &servers(&estate),
            Some(json!({ "slug": format!("srv-{index}"), "name": "S", "url": "https://x.example/mcp" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (eleventh, _) = call(
        &app,
        Method::POST,
        &servers(&estate),
        Some(json!({ "slug": "eleven", "name": "S", "url": "https://x.example/mcp" })),
    )
    .await;
    estate.forget(&pool).await;

    assert_eq!(twice, StatusCode::CONFLICT);
    assert_eq!(eleventh, StatusCode::CONFLICT);
}

#[tokio::test]
async fn only_a_member_who_manages_instances_changes_servers_and_keycloak_has_none() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let member = app(&pool, &estate.member, false);
    let owner = app(&pool, &estate.owner, false);
    let keycloak = format!(
        "/organisations/{}/deployments/{}/agents/servers",
        estate.organisation, estate.keycloak
    );

    let (member_status, _) = call(&member, Method::POST, &servers(&estate), Some(crm())).await;
    let (keycloak_status, _) = call(&owner, Method::POST, &keycloak, Some(crm())).await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(member_status, StatusCode::FORBIDDEN);
    assert_eq!(keycloak_status, StatusCode::CONFLICT);
    assert!(sent.is_empty());
}

#[tokio::test]
async fn an_agent_cannot_register_or_read_servers() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let agent = app(&pool, &estate.owner, true);
    let owner = app(&pool, &estate.owner, false);
    let (_, created) = call(&owner, Method::POST, &servers(&estate), Some(crm())).await;
    let id = created["data"]["id"].as_str().unwrap().to_string();

    let attempts = [
        (Method::GET, servers(&estate), None),
        (Method::POST, servers(&estate), Some(crm())),
        (
            Method::PUT,
            format!("{}/{id}", servers(&estate)),
            Some(json!({ "name": "x", "url": "https://x.example/", "enabled": true })),
        ),
        (Method::DELETE, format!("{}/{id}", servers(&estate)), None),
        (
            Method::POST,
            format!("{}/{id}/discover", servers(&estate)),
            None,
        ),
    ];
    let mut refused = Vec::new();
    for (method, uri, body) in attempts {
        let (status, answer) = call(&agent, method, &uri, body).await;
        refused.push((status, answer["code"].clone()));
    }
    let (_, listed) = call(&owner, Method::GET, &servers(&estate), None).await;
    estate.forget(&pool).await;

    for (status, code) in refused {
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(code, "E_DELEGATED_ROUTE_FORBIDDEN");
    }
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn removing_a_server_leaves_the_policies_that_name_it() {
    let Some(pool) = pool().await else {
        return;
    };
    let estate = Estate::seed(&pool, true).await;
    let app = app(&pool, &estate.owner, false);
    let (_, created) = call(&app, Method::POST, &servers(&estate), Some(crm())).await;
    let id = created["data"]["id"].as_str().unwrap().to_string();
    call(
        &app,
        Method::POST,
        &format!("{}/policies", estate.agents()),
        Some(json!({
            "name": "crm read",
            "enabled": true,
            "rule": { "effect": "permit" },
            "cedar": "permit(principal, action, resource) when { resource.server == \"crm\" };",
        })),
    )
    .await;

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("{}/{id}", servers(&estate)),
        None,
    )
    .await;
    let (_, policies) = call(
        &app,
        Method::GET,
        &format!("{}/policies", estate.agents()),
        None,
    )
    .await;
    let sent = estate.sent(&pool).await;
    estate.forget(&pool).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(policies["data"].as_array().unwrap().len(), 1);
    let last = sent.last().unwrap();
    assert_eq!(last["servers"], json!([]));
    assert_eq!(last["policies"].as_array().unwrap().len(), 1);
}
