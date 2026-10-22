use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::{
        actions::list_actions::list_actions_handler,
        dataplanes::{
            ack_actions::ack_actions_handler, claim_actions::claim_actions_handler,
            report_outcome::report_outcome_handler,
        },
        deployments::webhooks::{
            create_webhook_handler, delete_webhook_handler, get_webhook_handler,
            list_webhook_deliveries_handler, list_webhook_events_handler,
            list_webhook_realms_handler, list_webhooks_handler, retry_webhook_delivery_handler,
            reveal_webhook_secret_handler, rotate_webhook_secret_handler, update_webhook_handler,
        },
    },
    quickwit::QuickwitLogSearchIndex,
    state::AppState,
};
use autharie_auth::{Client, Identity, User};
use autharie_core::AutharieService;
use autharie_transit::{TransitConfig, TransitKeyProvider};
use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use axum_extra::routing::RouterExt;
use httpmock::{Method::POST, MockServer};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const KEY_32_A: &str = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE=";
const HEADER_VALUE: &str = "header-canary-0123456789";
const SECRET: &str = "whsec_canary_0123456789";
const VIEW_INSTANCES: i64 = 1 << 2;

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

fn transit(server: &MockServer) -> TransitKeyProvider {
    server.mock(|when, then| {
        when.method(POST).path("/v1/sys/mounts/transit");
        then.status(204);
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/transit/keys/cloud-credentials");
        then.status(204);
    });
    server.mock(|when, then| {
        when.method(POST)
            .path("/v1/transit/datakey/plaintext/cloud-credentials");
        then.status(200).json_body(json!({
            "data": {"plaintext": KEY_32_A, "ciphertext": "vault:v1:wrapped", "key_version": 1}
        }));
    });
    server.mock(|when, then| {
        when.method(POST)
            .path("/v1/transit/decrypt/cloud-credentials");
        then.status(200)
            .json_body(json!({"data": {"plaintext": KEY_32_A}}));
    });

    TransitKeyProvider::new(TransitConfig {
        address: server.base_url(),
        token: "root".to_string(),
        mount: "transit".to_string(),
        provider: autharie_core::backups::keys::ProviderName::platform(),
    })
    .expect("a provider")
}

struct Env {
    pool: PgPool,
    transit: MockServer,
    quickwit: MockServer,
    organisation: Uuid,
    owner: Uuid,
    viewer: Uuid,
    dataplane: Uuid,
    deployment: Uuid,
    herald: String,
}

impl Env {
    async fn new(pool: PgPool, plan: &str, kind: &str) -> Self {
        let env = Self {
            pool,
            transit: MockServer::start(),
            quickwit: MockServer::start(),
            organisation: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            viewer: Uuid::new_v4(),
            dataplane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            herald: format!("herald-{}", Uuid::new_v4()),
        };
        let tag = format!("wh-{}", env.organisation);

        for user in [env.owner, env.viewer] {
            sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'wh', $3)")
                .bind(user)
                .bind(format!("{user}@webhooks-api.test"))
                .bind(user.to_string())
                .execute(&env.pool)
                .await
                .expect("a user");
        }
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, 'active', $4, 5, 5, 5, now(), now())",
        )
        .bind(env.organisation)
        .bind(&tag)
        .bind(env.owner)
        .bind(plan)
        .execute(&env.pool)
        .await
        .expect("an organisation");
        sqlx::query(
            "INSERT INTO data_planes \
             (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
              capacity_storage_gib, created_at, updated_at, herald_client_id, herald_subject) \
             VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200, now(), now(), $3, $3)",
        )
        .bind(env.dataplane)
        .bind(&tag)
        .bind(&env.herald)
        .execute(&env.pool)
        .await
        .expect("a data plane");
        sqlx::query(
            "INSERT INTO deployments \
             (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
              cpu_millis, memory_mib, storage_gib, created_by, created_at, updated_at, hostname_slug) \
             VALUES ($1, $2, $3, 'auth', $6, 'failed', $4, '26.0.0', \
                     500, 1024, 1, $5, now(), now(), $4)",
        )
        .bind(env.deployment)
        .bind(env.organisation)
        .bind(env.dataplane)
        .bind(&tag)
        .bind(env.owner)
        .bind(kind)
        .execute(&env.pool)
        .await
        .expect("a deployment");

        env.grant_viewer().await;
        env
    }

    async fn grant_viewer(&self) {
        let member = Uuid::new_v4();
        let role = Uuid::new_v4();
        sqlx::query("INSERT INTO members (id, user_id, organisation_id) VALUES ($1, $2, $3)")
            .bind(member)
            .bind(self.viewer)
            .bind(self.organisation)
            .execute(&self.pool)
            .await
            .expect("a member");
        sqlx::query(
            "INSERT INTO roles (id, name, permissions, organisation_id) VALUES ($1, 'viewer', $2, $3)",
        )
        .bind(role)
        .bind(VIEW_INSTANCES)
        .bind(self.organisation)
        .execute(&self.pool)
        .await
        .expect("a role");
        sqlx::query("INSERT INTO member_roles (member_id, role_id) VALUES ($1, $2)")
            .bind(member)
            .bind(role)
            .execute(&self.pool)
            .await
            .expect("a grant");
    }

    async fn forget(&self) {
        for (statement, id) in [
            ("DELETE FROM deployments WHERE id = $1", self.deployment),
            ("DELETE FROM data_planes WHERE id = $1", self.dataplane),
            ("DELETE FROM organisations WHERE id = $1", self.organisation),
            ("DELETE FROM users WHERE id = $1", self.owner),
            ("DELETE FROM users WHERE id = $1", self.viewer),
        ] {
            sqlx::query(statement)
                .bind(id)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
    }

    fn state(&self) -> AppState {
        AppState {
            args: Arc::new(Args::default()),
            service: AutharieService::new(self.pool.clone())
                .with_credential_keys(Some(transit(&self.transit))),
            certificate_source: None,
            quickwit_search: Some(Arc::new(QuickwitLogSearchIndex::new(
                self.quickwit.base_url(),
            ))),
            quickwit_traces: None,
            billing: None,
        }
    }

    fn app(&self, identity: Identity) -> Router {
        Router::new()
            .typed_get(list_webhooks_handler)
            .typed_post(create_webhook_handler)
            .typed_get(get_webhook_handler)
            .typed_put(update_webhook_handler)
            .typed_delete(delete_webhook_handler)
            .typed_post(rotate_webhook_secret_handler)
            .typed_get(reveal_webhook_secret_handler)
            .typed_get(list_webhook_deliveries_handler)
            .typed_post(retry_webhook_delivery_handler)
            .typed_get(list_webhook_realms_handler)
            .typed_get(list_webhook_events_handler)
            .typed_get(list_actions_handler)
            .typed_post(report_outcome_handler)
            .typed_post(claim_actions_handler)
            .typed_post(ack_actions_handler)
            .layer(Extension(identity))
            .with_state(self.state())
    }

    fn owner_app(&self) -> Router {
        self.app(user(self.owner))
    }

    fn viewer_app(&self) -> Router {
        self.app(user(self.viewer))
    }

    fn herald_app(&self) -> Router {
        self.app(Identity::Client(Client {
            id: self.herald.clone(),
            client_id: "herald".to_string(),
            roles: vec![],
            scopes: vec![],
        }))
    }

    fn webhooks(&self) -> String {
        format!(
            "/organisations/{}/deployments/{}/webhooks",
            self.organisation, self.deployment
        )
    }

    fn webhook(&self, id: &str) -> String {
        format!("{}/{id}", self.webhooks())
    }

    fn outcome(&self) -> String {
        format!(
            "/dataplanes/{}/deployments/{}/outcome",
            self.dataplane, self.deployment
        )
    }

    async fn create(&self, name: &str) -> String {
        let (status, body) = call(
            &self.owner_app(),
            Method::POST,
            &self.webhooks(),
            Some(webhook_body(name)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        body["data"]["id"].as_str().expect("an id").to_string()
    }

    async fn report(&self, body: Value) -> StatusCode {
        call(
            &self.herald_app(),
            Method::POST,
            &self.outcome(),
            Some(body),
        )
        .await
        .0
    }

    async fn actions_of_type(&self, action_type: &str) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT payload FROM actions WHERE deployment_id = $1 AND action_type = $2 \
             ORDER BY created_at",
        )
        .bind(self.deployment)
        .bind(action_type)
        .fetch_all(&self.pool)
        .await
        .expect("actions")
    }

    async fn audited(&self, action: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_log WHERE organisation_id = $1 AND action = $2",
        )
        .bind(self.organisation)
        .bind(action)
        .fetch_one(&self.pool)
        .await
        .expect("a count")
    }
}

fn user(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "wh".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

fn webhook_body(name: &str) -> Value {
    json!({
        "name": name,
        "endpoint": "https://example.com/hooks/autharie",
        "events": ["user.created"],
    })
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

fn active(id: &str, ferriskey_id: &str) -> Value {
    json!({ "id": id, "ferriskey_id": ferriskey_id, "phase": "Active", "secret_version": 1 })
}

#[tokio::test]
async fn a_member_creates_a_webhook_and_receives_its_secret_spec_wh_1() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let (status, created) = call(
        &env.owner_app(),
        Method::POST,
        &env.webhooks(),
        Some(webhook_body("orders")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["data"]["state"]["phase"], "Pending");
    assert_eq!(created["data"]["realm"], "master");
    let id = created["data"]["id"].as_str().unwrap().to_string();

    let sent = env.actions_of_type("deployment.webhooks").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["revision"], 1);
    assert_eq!(sent[0]["webhooks"][0]["id"], id);

    let (status, secret) = call(
        &env.owner_app(),
        Method::GET,
        &format!("{}/secret", env.webhook(&id)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{secret}");
    assert_eq!(env.audited("deployment.webhook.secret_revealed").await, 0);

    let reported = env
        .report(json!({
            "kind": "webhooks",
            "revision": 1,
            "webhooks": [active(&id, "fk-1")],
            "secrets": [{ "id": id, "version": 1, "secret": SECRET }],
            "realms": ["master"],
            "requests": []
        }))
        .await;
    assert_eq!(reported, StatusCode::OK);

    let (_, shown) = call(&env.owner_app(), Method::GET, &env.webhook(&id), None).await;
    assert_eq!(shown["data"]["state"]["phase"], "Active");
    assert_eq!(shown["data"]["state"]["ferriskey_id"], "fk-1");
    assert_eq!(shown["data"]["secret_version"], 1);
    assert!(!shown.to_string().contains(SECRET));

    let (status, secret) = call(
        &env.owner_app(),
        Method::GET,
        &format!("{}/secret", env.webhook(&id)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{secret}");
    assert_eq!(secret["data"]["secret"], SECRET);
    assert_eq!(secret["data"]["version"], 1);
    assert_eq!(env.audited("deployment.webhook.secret_revealed").await, 1);
    assert_eq!(env.audited("deployment.webhook.created").await, 1);

    let stored: Vec<u8> = sqlx::query_scalar(
        "SELECT ciphertext FROM deployment_webhook_secrets WHERE webhook_id = $1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&env.pool)
    .await
    .expect("a stored secret");
    assert!(
        !String::from_utf8_lossy(&stored).contains(SECRET),
        "the secret is stored in clear"
    );

    env.forget().await;
}

#[tokio::test]
async fn rotating_the_secret_replaces_it_and_bumps_its_version_spec_wh_4() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let id = env.create("orders").await;
    env.report(json!({
        "kind": "webhooks", "revision": 1, "webhooks": [active(&id, "fk-1")],
        "secrets": [{ "id": id, "version": 1, "secret": SECRET }], "realms": ["master"]
    }))
    .await;

    let (status, rotated) = call(
        &env.owner_app(),
        Method::POST,
        &format!("{}/secret/rotate", env.webhook(&id)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{rotated}");
    let request_id = rotated["data"]["id"].as_str().unwrap().to_string();

    let sent = env
        .actions_of_type("deployment.webhook.rotate_secret")
        .await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["webhook_id"], id);
    assert_eq!(sent[0]["request_id"], request_id);
    assert_eq!(env.audited("deployment.webhook.secret_rotated").await, 1);

    let reported = env
        .report(json!({
            "kind": "webhooks", "revision": 1, "webhooks": [active(&id, "fk-1")],
            "secrets": [
                { "id": id, "version": 1, "secret": "whsec_replayed" },
                { "id": id, "version": 2, "secret": "whsec_second" }
            ],
            "realms": ["master"],
            "requests": [{ "id": request_id, "phase": "Done" }]
        }))
        .await;
    assert_eq!(reported, StatusCode::OK);

    let (_, secret) = call(
        &env.owner_app(),
        Method::GET,
        &format!("{}/secret", env.webhook(&id)),
        None,
    )
    .await;
    assert_eq!(secret["data"]["secret"], "whsec_second");
    assert_eq!(secret["data"]["version"], 2);

    let (_, shown) = call(&env.owner_app(), Method::GET, &env.webhook(&id), None).await;
    assert_eq!(shown["data"]["requests"][0]["phase"], "done");

    env.forget().await;
}

#[tokio::test]
async fn a_free_deployment_cannot_hold_a_sixth_webhook_spec_wh_5() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "free", "ferriskey").await;
    for index in 0..5 {
        env.create(&format!("hook-{index}")).await;
    }

    let (status, refused) = call(
        &env.owner_app(),
        Method::POST,
        &env.webhooks(),
        Some(webhook_body("sixth")),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert!(refused.to_string().contains("webhooks"));
    assert_eq!(env.actions_of_type("deployment.webhooks").await.len(), 5);

    env.forget().await;
}

#[tokio::test]
async fn an_http_endpoint_or_an_unknown_event_is_refused_spec_wh_6() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let mut http = webhook_body("orders");
    http["endpoint"] = json!("http://example.com/hook");
    let mut unknown = webhook_body("orders");
    unknown["events"] = json!(["user.exploded"]);
    let mut other_realm = webhook_body("orders");
    other_realm["realm"] = json!("ghosts");

    for body in [http, unknown, other_realm] {
        let (status, refused) =
            call(&env.owner_app(), Method::POST, &env.webhooks(), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    }

    let (_, listed) = call(&env.owner_app(), Method::GET, &env.webhooks(), None).await;
    assert_eq!(listed["data"], json!([]));
    assert!(env.actions_of_type("deployment.webhooks").await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn keycloak_deployments_have_no_webhooks() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "keycloak").await;

    let (status, refused) = call(
        &env.owner_app(),
        Method::POST,
        &env.webhooks(),
        Some(webhook_body("orders")),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    env.forget().await;
}

#[tokio::test]
async fn deliveries_are_listed_from_the_index_and_a_failed_one_can_be_retried_spec_wh_7() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let id = env.create("orders").await;

    let search = env.quickwit.mock(|when, then| {
        when.method(POST)
            .path(format!("/api/v1/logs-{}/search", env.organisation))
            .body_contains("kind:webhook_delivery")
            .body_contains(env.deployment.to_string())
            .body_contains(&id)
            .body_contains("kind:webhook_delivery");
        then.status(200).json_body(json!({
            "num_hits": 3,
            "hits": [
                {
                    "kind": "webhook_delivery", "timestamp": "2026-10-22T10:00:00Z",
                    "deployment_id": env.deployment, "webhook_id": id,
                    "delivery_id": "delivery-1", "event": "user.created", "status": "failed",
                    "attempt_count": 3, "last_status_code": 500, "last_error": "boom",
                    "created_at": "2026-10-22T09:59:00Z", "updated_at": "2026-10-22T10:00:00Z"
                },
                {
                    "kind": "webhook_delivery", "timestamp": "2026-10-22T09:59:30Z",
                    "deployment_id": env.deployment, "webhook_id": id,
                    "delivery_id": "delivery-1", "event": "user.created", "status": "pending",
                    "attempt_count": 0,
                    "created_at": "2026-10-22T09:59:00Z", "updated_at": "2026-10-22T09:59:30Z"
                },
                {
                    "kind": "webhook_delivery", "timestamp": "2026-10-22T09:00:00Z",
                    "deployment_id": env.deployment, "webhook_id": id,
                    "delivery_id": "delivery-0", "event": "user.created", "status": "failed",
                    "attempt_count": 1,
                    "created_at": "2026-10-22T08:59:00Z", "updated_at": "2026-10-22T09:00:00Z"
                }
            ]
        }));
    });

    let (status, listed) = call(
        &env.viewer_app(),
        Method::GET,
        &format!("{}/deliveries?status=failed", env.webhook(&id)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    search.assert();
    assert_eq!(listed["data"][0]["delivery_id"], "delivery-1");
    assert_eq!(listed["data"][0]["status"], "failed");
    assert_eq!(listed["data"][0]["last_status_code"], 500);
    assert_eq!(listed["data"][1]["delivery_id"], "delivery-0");
    assert_eq!(listed["data"].as_array().unwrap().len(), 2);

    let (status, retried) = call(
        &env.owner_app(),
        Method::POST,
        &format!("{}/deliveries/delivery-1/retry", env.webhook(&id)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{retried}");
    let sent = env
        .actions_of_type("deployment.webhook.retry_delivery")
        .await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["delivery_id"], "delivery-1");
    assert_eq!(sent[0]["webhook_id"], id);
    assert_eq!(sent[0]["request_id"], retried["data"]["id"]);

    env.forget().await;
}

#[tokio::test]
async fn an_index_that_holds_no_deliveries_answers_an_empty_list() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let id = env.create("orders").await;

    env.quickwit.mock(|when, then| {
        when.method(POST)
            .path(format!("/api/v1/logs-{}/search", env.organisation));
        then.status(404).body("index not found");
    });

    let (status, listed) = call(
        &env.owner_app(),
        Method::GET,
        &format!("{}/deliveries", env.webhook(&id)),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"], json!([]));

    env.forget().await;
}

#[tokio::test]
async fn a_viewer_sees_webhooks_but_cannot_reveal_rotate_change_or_retry_spec_wh_8() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let id = env.create("orders").await;
    env.report(json!({
        "kind": "webhooks", "revision": 1, "webhooks": [active(&id, "fk-1")],
        "secrets": [{ "id": id, "version": 1, "secret": SECRET }], "realms": ["master"]
    }))
    .await;
    let viewer = env.viewer_app();

    let (status, listed) = call(&viewer, Method::GET, &env.webhooks(), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"][0]["id"], id);
    for uri in [
        env.webhook(&id),
        format!(
            "{}/webhook-realms",
            env.webhooks().trim_end_matches("/webhooks")
        ),
        "/webhook-events".to_string(),
    ] {
        let (status, body) = call(&viewer, Method::GET, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }

    let attempts = [
        (Method::GET, format!("{}/secret", env.webhook(&id)), None),
        (
            Method::POST,
            format!("{}/secret/rotate", env.webhook(&id)),
            None,
        ),
        (Method::POST, env.webhooks(), Some(webhook_body("new"))),
        (Method::PUT, env.webhook(&id), Some(webhook_body("changed"))),
        (Method::DELETE, env.webhook(&id), None),
        (
            Method::POST,
            format!("{}/deliveries/delivery-1/retry", env.webhook(&id)),
            None,
        ),
    ];
    for (method, uri, body) in attempts {
        let (status, refused) = call(&viewer, method.clone(), &uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {refused}");
    }

    assert_eq!(env.audited("deployment.webhook.secret_revealed").await, 0);
    assert_eq!(env.actions_of_type("deployment.webhooks").await.len(), 1);

    env.forget().await;
}

#[tokio::test]
async fn an_older_outcome_redelivered_after_a_newer_one_changes_nothing_spec_wh_9() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let id = env.create("orders").await;

    let (status, updated) = call(
        &env.owner_app(),
        Method::PUT,
        &env.webhook(&id),
        Some(webhook_body("renamed")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["data"]["state"]["phase"], "Pending");
    let revisions: Vec<u64> = env
        .actions_of_type("deployment.webhooks")
        .await
        .iter()
        .map(|payload| payload["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, vec![1, 2]);

    env.report(json!({
        "kind": "webhooks", "revision": 2, "webhooks": [active(&id, "fk-2")], "realms": ["master"]
    }))
    .await;
    let stale = env
        .report(json!({
            "kind": "webhooks", "revision": 1,
            "webhooks": [{ "id": id, "phase": "Failed", "message": "old news" }],
            "realms": ["master", "ghosts"]
        }))
        .await;
    assert_eq!(stale, StatusCode::OK);

    let (_, shown) = call(&env.owner_app(), Method::GET, &env.webhook(&id), None).await;
    assert_eq!(shown["data"]["state"]["phase"], "Active");
    assert_eq!(shown["data"]["state"]["ferriskey_id"], "fk-2");
    let (_, realms) = call(
        &env.owner_app(),
        Method::GET,
        &format!(
            "/organisations/{}/deployments/{}/webhook-realms",
            env.organisation, env.deployment
        ),
        None,
    )
    .await;
    assert_eq!(realms["data"], json!(["master"]));

    env.forget().await;
}

#[tokio::test]
async fn header_values_never_appear_in_a_response_and_leave_the_action_once_published_spec_wh_10() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let mut body = webhook_body("orders");
    body["headers"] = json!([{ "name": "x-api-key", "value": HEADER_VALUE }]);
    let (status, created) = call(&env.owner_app(), Method::POST, &env.webhooks(), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["data"]["headers"], json!(["x-api-key"]));

    let mut change = webhook_body("orders");
    change["headers"] = json!([{ "name": "x-api-key" }]);
    let (status, updated) = call(
        &env.owner_app(),
        Method::PUT,
        &env.webhook(&id),
        Some(change),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let actions_uri = format!(
        "/organisations/{}/deployments/{}/actions",
        env.organisation, env.deployment
    );
    let mut every_response = vec![created.to_string(), updated.to_string()];
    for uri in [env.webhooks(), env.webhook(&id), actions_uri] {
        let (status, body) = call(&env.owner_app(), Method::GET, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        every_response.push(body.to_string());
    }
    for response in &every_response {
        assert!(!response.contains(HEADER_VALUE), "{response}");
    }

    let sent = env.actions_of_type("deployment.webhooks").await;
    assert_eq!(
        sent[1]["headers"][&id]["x-api-key"], HEADER_VALUE,
        "an update without a value keeps the stored one on its way to the data plane"
    );

    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT headers_ciphertext FROM deployment_webhooks WHERE id = $1")
            .bind(Uuid::parse_str(&id).unwrap())
            .fetch_one(&env.pool)
            .await
            .expect("sealed headers");
    assert!(!String::from_utf8_lossy(&sealed).contains(HEADER_VALUE));

    let (status, claimed) = call(
        &env.herald_app(),
        Method::POST,
        &format!("/dataplanes/{}/actions:claim", env.dataplane),
        Some(json!({ "deployment_ids": [env.deployment], "max": 10, "lease_seconds": 60 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let claimed = claimed["data"].as_array().unwrap();
    assert_eq!(claimed.len(), 2);
    assert!(
        claimed
            .iter()
            .all(|action| action["payload"]["data"]["headers"].is_object()),
        "the data plane gets the values on the claim"
    );

    let published: Vec<&str> = claimed
        .iter()
        .map(|action| action["id"].as_str().unwrap())
        .collect();
    let (status, acked) = call(
        &env.herald_app(),
        Method::POST,
        &format!(
            "/dataplanes/{}/deployments/{}/actions:ack",
            env.dataplane, env.deployment
        ),
        Some(json!({ "published": published, "failed": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{acked}");

    let stored = env.actions_of_type("deployment.webhooks").await;
    assert_eq!(stored.len(), 2);
    for payload in stored {
        assert!(payload.get("headers").is_none(), "{payload}");
        assert!(!payload.to_string().contains(HEADER_VALUE));
        assert!(payload["webhooks"][0]["header_names"] == json!(["x-api-key"]));
    }

    env.forget().await;
}

#[tokio::test]
async fn the_outcome_endpoint_still_takes_a_deployments_own_outcome() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let (status, body) = call(
        &env.herald_app(),
        Method::POST,
        &env.outcome(),
        Some(json!({ "outcome": "running", "version": "26.0.0" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = call(
        &env.herald_app(),
        Method::POST,
        &env.outcome(),
        Some(json!({ "kind": "mystery" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = call(
        &env.herald_app(),
        Method::POST,
        &env.outcome(),
        Some(json!({ "kind": "webhooks" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    env.forget().await;
}

#[tokio::test]
async fn an_outcome_from_another_data_plane_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let stranger = Identity::Client(Client {
        id: "herald-someone-else".to_string(),
        client_id: "herald".to_string(),
        roles: vec![],
        scopes: vec![],
    });

    let (status, body) = call(
        &env.app(stranger),
        Method::POST,
        &env.outcome(),
        Some(json!({ "kind": "webhooks", "revision": 1 })),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    env.forget().await;
}

#[tokio::test]
async fn deleting_a_webhook_sends_the_remaining_set() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let first = env.create("first").await;
    let second = env.create("second").await;

    let (status, _) = call(&env.owner_app(), Method::DELETE, &env.webhook(&first), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let sent = env.actions_of_type("deployment.webhooks").await;
    let last = sent.last().unwrap();
    assert_eq!(last["revision"], 3);
    assert_eq!(last["webhooks"].as_array().unwrap().len(), 1);
    assert_eq!(last["webhooks"][0]["id"], second);

    let (status, _) = call(&env.owner_app(), Method::GET, &env.webhook(&first), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(env.audited("deployment.webhook.deleted").await, 1);

    env.forget().await;
}
