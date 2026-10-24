use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::{
        actions::{get_action::get_action_handler, list_actions::list_actions_handler},
        dataplanes::{
            ack_actions::ack_actions_handler, claim_actions::claim_actions_handler,
            report_outcome::report_outcome_handler,
        },
        deployments::iam::{create_iam_request_handler, get_iam_request_handler},
    },
    state::AppState,
};
use autharie_auth::{Client, Identity, User};
use autharie_core::AutharieService;
use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use axum_extra::routing::RouterExt;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const VIEW_ORGANISATION: i64 = 1;
const VIEW_INSTANCES: i64 = 1 << 2;
const TERM: &str = "alice-the-canary";

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

struct Env {
    pool: PgPool,
    organisation: Uuid,
    owner: Uuid,
    viewer: Uuid,
    plain: Uuid,
    dataplane: Uuid,
    intruder_dataplane: Uuid,
    deployment: Uuid,
    herald: String,
    intruder: String,
}

impl Env {
    async fn new(pool: PgPool, kind: &str) -> Self {
        let env = Self {
            pool,
            organisation: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            viewer: Uuid::new_v4(),
            plain: Uuid::new_v4(),
            dataplane: Uuid::new_v4(),
            intruder_dataplane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            herald: format!("herald-{}", Uuid::new_v4()),
            intruder: format!("herald-{}", Uuid::new_v4()),
        };
        let tag = format!("iam-{}", env.organisation);

        for user in [env.owner, env.viewer, env.plain] {
            sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'iam', $3)")
                .bind(user)
                .bind(format!("{user}@iam-api.test"))
                .bind(user.to_string())
                .execute(&env.pool)
                .await
                .expect("a user");
        }
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, 'active', 'starter', 5, 5, 5, now(), now())",
        )
        .bind(env.organisation)
        .bind(&tag)
        .bind(env.owner)
        .execute(&env.pool)
        .await
        .expect("an organisation");
        for (dataplane, herald) in [
            (env.dataplane, &env.herald),
            (env.intruder_dataplane, &env.intruder),
        ] {
            sqlx::query(
                "INSERT INTO data_planes \
                 (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
                  capacity_storage_gib, created_at, updated_at, herald_client_id, herald_subject) \
                 VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200, now(), now(), $3, $3)",
            )
            .bind(dataplane)
            .bind(format!("{tag}-{dataplane}"))
            .bind(herald)
            .execute(&env.pool)
            .await
            .expect("a data plane");
        }
        sqlx::query(
            "INSERT INTO deployments \
             (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
              cpu_millis, memory_mib, storage_gib, created_by, created_at, updated_at, hostname_slug) \
             VALUES ($1, $2, $3, 'auth', $6, 'successful', $4, '26.0.0', \
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

        env.grant(env.viewer, VIEW_INSTANCES).await;
        env.grant(env.plain, VIEW_ORGANISATION).await;
        env
    }

    async fn grant(&self, user: Uuid, permissions: i64) {
        let member = Uuid::new_v4();
        let role = Uuid::new_v4();
        sqlx::query("INSERT INTO members (id, user_id, organisation_id) VALUES ($1, $2, $3)")
            .bind(member)
            .bind(user)
            .bind(self.organisation)
            .execute(&self.pool)
            .await
            .expect("a member");
        sqlx::query(
            "INSERT INTO roles (id, name, permissions, organisation_id) VALUES ($1, $1::text, $2, $3)",
        )
        .bind(role)
        .bind(permissions)
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
            (
                "DELETE FROM data_planes WHERE id = $1",
                self.intruder_dataplane,
            ),
            ("DELETE FROM organisations WHERE id = $1", self.organisation),
            ("DELETE FROM users WHERE id = $1", self.owner),
            ("DELETE FROM users WHERE id = $1", self.viewer),
            ("DELETE FROM users WHERE id = $1", self.plain),
        ] {
            sqlx::query(statement)
                .bind(id)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
    }

    fn app(&self, identity: Identity) -> Router {
        Router::new()
            .typed_post(create_iam_request_handler)
            .typed_get(get_iam_request_handler)
            .typed_get(list_actions_handler)
            .typed_get(get_action_handler)
            .typed_post(report_outcome_handler)
            .typed_post(claim_actions_handler)
            .typed_post(ack_actions_handler)
            .layer(Extension(identity))
            .with_state(AppState {
                args: Arc::new(Args::default()),
                service: AutharieService::new(self.pool.clone()),
                certificate_source: None,
                quickwit_search: None,
                quickwit_traces: None,
                billing: None,
            })
    }

    fn owner_app(&self) -> Router {
        self.app(user(self.owner))
    }

    fn viewer_app(&self) -> Router {
        self.app(user(self.viewer))
    }

    fn plain_app(&self) -> Router {
        self.app(user(self.plain))
    }

    fn herald_app(&self) -> Router {
        self.app(client(&self.herald))
    }

    fn requests(&self) -> String {
        format!(
            "/organisations/{}/deployments/{}/iam/requests",
            self.organisation, self.deployment
        )
    }

    fn request(&self, id: &str) -> String {
        format!("{}/{id}", self.requests())
    }

    fn actions(&self) -> String {
        format!(
            "/organisations/{}/deployments/{}/actions",
            self.organisation, self.deployment
        )
    }

    fn outcome(&self) -> String {
        format!(
            "/dataplanes/{}/deployments/{}/outcome",
            self.dataplane, self.deployment
        )
    }

    async fn ask(&self, app: &Router, body: Value) -> (StatusCode, Value) {
        call(app, Method::POST, &self.requests(), Some(body)).await
    }

    async fn ask_list(&self) -> String {
        let (status, body) = self.ask(&self.owner_app(), list_body("master", None)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");

        body["data"]["request_id"]
            .as_str()
            .expect("a request id")
            .to_string()
    }

    async fn poll(&self, app: &Router, id: &str) -> (StatusCode, Value) {
        call(app, Method::GET, &self.request(id), None).await
    }

    async fn report(&self, app: &Router, body: Value) -> (StatusCode, Value) {
        call(app, Method::POST, &self.outcome(), Some(body)).await
    }

    async fn report_done(&self, id: &str, envelope: Value) -> (StatusCode, Value) {
        self.report(
            &self.herald_app(),
            json!({
                "deployment_id": self.deployment,
                "kind": "iam",
                "request_id": id,
                "phase": "Done",
                "envelope": envelope
            }),
        )
        .await
    }

    async fn report_failed(&self, id: &str, message: &str) -> (StatusCode, Value) {
        self.report(
            &self.herald_app(),
            json!({
                "deployment_id": self.deployment,
                "kind": "iam",
                "request_id": id,
                "phase": "Failed",
                "message": message
            }),
        )
        .await
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE {} = $1",
            if table == "audit_log" {
                "organisation_id"
            } else {
                "deployment_id"
            }
        ))
        .bind(if table == "audit_log" {
            self.organisation
        } else {
            self.deployment
        })
        .fetch_one(&self.pool)
        .await
        .expect("a count")
    }

    async fn iam_actions(&self) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT payload FROM actions WHERE deployment_id = $1 \
             AND action_type = 'deployment.iam.request' ORDER BY created_at",
        )
        .bind(self.deployment)
        .fetch_all(&self.pool)
        .await
        .expect("actions")
    }

    async fn nothing_recorded(&self) -> bool {
        self.count("iam_requests").await == 0
            && self.count("actions").await == 0
            && self.count("audit_log").await == 0
    }
}

fn user(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "iam".to_string(),
        email: None,
        name: None,
        roles: vec![],
        scopes: vec![],
        act: None,
    })
}

fn client(id: &str) -> Identity {
    Identity::Client(Client {
        id: id.to_string(),
        client_id: "herald".to_string(),
        roles: vec![],
        scopes: vec![],
        act: None,
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

fn list_body(realm: &str, search: Option<&str>) -> Value {
    let mut operation = json!({ "type": "applications.list" });
    if let Some(search) = search {
        operation["search"] = json!(search);
    }

    json!({ "realm": realm, "operation": operation, "sealing_key": jwk(256) })
}

fn envelope() -> Value {
    json!({
        "v": 1,
        "alg": "RSA-OAEP-256+A256GCM",
        "ek": URL_SAFE_NO_PAD.encode([1u8; 256]),
        "iv": URL_SAFE_NO_PAD.encode([2u8; 12]),
        "ct": URL_SAFE_NO_PAD.encode([3u8; 96]),
    })
}

#[tokio::test]
async fn spec_iam_1_a_member_gets_202_then_the_envelope_once_then_410() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;

    let (status, body) = env.ask(&env.viewer_app(), list_body("master", None)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let id = body["data"]["request_id"].as_str().unwrap().to_string();

    let (waiting, waiting_body) = env.poll(&env.viewer_app(), &id).await;
    let (reported, _) = env.report_done(&id, envelope()).await;
    let (first, first_body) = env.poll(&env.viewer_app(), &id).await;
    let (second, _) = env.poll(&env.viewer_app(), &id).await;
    env.forget().await;

    assert_eq!(waiting, StatusCode::ACCEPTED);
    assert_eq!(waiting_body["data"]["status"], "pending");
    assert_eq!(reported, StatusCode::OK);
    assert_eq!(first, StatusCode::OK);
    assert_eq!(first_body["data"]["envelope"], envelope());
    assert_eq!(second, StatusCode::GONE);
}

#[tokio::test]
async fn the_action_carries_the_normalised_operation_the_sealing_key_and_the_expiry() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let sealing_key = jwk(256);

    let (status, body) = env
        .ask(
            &env.owner_app(),
            json!({
                "realm": "master",
                "operation": { "type": "applications.list", "limit": 10, "search": TERM },
                "sealing_key": sealing_key
            }),
        )
        .await;
    let id = body["data"]["request_id"].as_str().unwrap().to_string();
    let (_, get_body) = env
        .ask(
            &env.owner_app(),
            json!({
                "realm": "master",
                "operation": { "type": "applications.get", "id": Uuid::nil() },
                "sealing_key": sealing_key
            }),
        )
        .await;
    let sent = env.iam_actions().await;
    env.forget().await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(get_body["data"]["request_id"].as_str().is_some());
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["deployment_id"], json!(env.deployment));
    assert_eq!(sent[0]["request_id"], id);
    assert_eq!(sent[0]["realm"], "master");
    assert!(sent[0]["namespace"].as_str().is_some());
    assert_eq!(
        sent[0]["operation"],
        json!({ "type": "applications.list", "page": 1, "limit": 10, "search": TERM })
    );
    assert_eq!(
        sent[0]["sealing_key"],
        json!({ "kty": "RSA", "n": sealing_key["n"], "e": "AQAB", "alg": "RSA-OAEP-256" })
    );
    assert!(chrono::DateTime::parse_from_rfc3339(sent[0]["expires_at"].as_str().unwrap()).is_ok());
    assert_eq!(
        sent[1]["operation"],
        json!({ "type": "applications.get", "id": Uuid::nil() })
    );
}

#[tokio::test]
async fn spec_iam_2_the_stored_value_is_the_envelope_received_and_is_gone_once_delivered() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let id = env.ask_list().await;
    env.report_done(&id, envelope()).await;

    let stored: Option<Value> =
        sqlx::query_scalar("SELECT envelope FROM iam_requests WHERE request_id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let row_text: String =
        sqlx::query_scalar("SELECT r::text FROM iam_requests r WHERE request_id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    env.poll(&env.owner_app(), &id).await;
    let after: Option<Value> =
        sqlx::query_scalar("SELECT envelope FROM iam_requests WHERE request_id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let tombstone: String =
        sqlx::query_scalar("SELECT phase FROM iam_requests WHERE request_id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    env.forget().await;

    assert_eq!(stored, Some(envelope()));
    assert!(!row_text.contains(TERM));
    assert_eq!(after, None);
    assert_eq!(tombstone, "delivered");
}

#[tokio::test]
async fn spec_iam_5_an_operation_that_does_not_exist_is_a_400_and_nothing_is_recorded() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let mut refused = Vec::new();

    for operation in [
        json!({ "type": "applications.delete", "id": Uuid::new_v4() }),
        json!({ "type": "GET /realms/master/clients" }),
        json!({ "type": "applications.get" }),
        json!({ "type": "applications.list", "limit": 51 }),
        json!({ "type": "applications.list", "page": 0 }),
        json!({}),
    ] {
        let (status, _) = env
            .ask(
                &env.owner_app(),
                json!({ "realm": "master", "operation": operation, "sealing_key": jwk(256) }),
            )
            .await;
        refused.push(status);
    }
    let (extra, _) = env
        .ask(
            &env.owner_app(),
            json!({ "realm": "master", "operation": { "type": "applications.list" },
                    "sealing_key": jwk(256), "path": "/admin/realms" }),
        )
        .await;
    let nothing = env.nothing_recorded().await;
    env.forget().await;

    assert!(
        refused
            .iter()
            .all(|status| *status == StatusCode::BAD_REQUEST)
    );
    assert_eq!(extra, StatusCode::BAD_REQUEST);
    assert!(nothing);
}

#[tokio::test]
async fn spec_iam_6_a_weak_or_wrong_key_is_a_400_and_nothing_is_recorded() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let operation = json!({ "type": "applications.list" });
    let mut private = jwk(256);
    private["d"] = json!("AAAA");
    let mut refused = Vec::new();

    for key in [
        jwk(255),
        json!({ "kty": "EC", "crv": "P-256", "x": "AA", "y": "AA", "n": "AA", "e": "AQAB" }),
        private,
    ] {
        let (status, _) = env
            .ask(
                &env.owner_app(),
                json!({ "realm": "master", "operation": operation, "sealing_key": key }),
            )
            .await;
        refused.push(status);
    }
    let nothing = env.nothing_recorded().await;
    env.forget().await;

    assert!(
        refused
            .iter()
            .all(|status| *status == StatusCode::BAD_REQUEST)
    );
    assert!(nothing);
}

#[tokio::test]
async fn a_realm_that_is_malformed_or_not_on_the_deployment_is_a_400_and_nothing_is_recorded() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;

    let (malformed, _) = env
        .ask(&env.owner_app(), list_body("bad realm", None))
        .await;
    let (unknown, _) = env
        .ask(&env.owner_app(), list_body("elsewhere", None))
        .await;
    let nothing = env.nothing_recorded().await;
    env.forget().await;

    assert_eq!(malformed, StatusCode::BAD_REQUEST);
    assert_eq!(unknown, StatusCode::BAD_REQUEST);
    assert!(nothing);
}

#[tokio::test]
async fn spec_iam_7_a_member_without_view_instances_is_a_403_and_nothing_is_recorded() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;

    let (asked, _) = env.ask(&env.plain_app(), list_body("master", None)).await;
    let id = env.ask_list().await;
    let (polled, _) = env.poll(&env.plain_app(), &id).await;
    let before = env.count("iam_requests").await;
    env.forget().await;

    assert_eq!(asked, StatusCode::FORBIDDEN);
    assert_eq!(polled, StatusCode::FORBIDDEN);
    assert_eq!(before, 1);
}

#[tokio::test]
async fn spec_iam_7_a_deployment_that_is_not_ferriskey_is_refused_as_the_other_features_are() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "keycloak").await;

    let (status, _) = env.ask(&env.owner_app(), list_body("master", None)).await;
    let nothing = env.nothing_recorded().await;
    env.forget().await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(nothing);
}

#[tokio::test]
async fn a_deployment_of_another_organisation_is_not_found() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;

    let (status, _) = call(
        &env.owner_app(),
        Method::POST,
        &format!(
            "/organisations/{}/deployments/{}/iam/requests",
            Uuid::new_v4(),
            env.deployment
        ),
        Some(list_body("master", None)),
    )
    .await;
    env.forget().await;

    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "{status}"
    );
}

#[tokio::test]
async fn spec_iam_8_every_request_is_audited_without_its_search_or_its_result() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;

    let (_, asked) = env
        .ask(&env.viewer_app(), list_body("master", Some(TERM)))
        .await;
    let id = asked["data"]["request_id"].as_str().unwrap().to_string();
    env.report_done(&id, envelope()).await;
    env.poll(&env.viewer_app(), &id).await;

    let rows = sqlx::query(
        "SELECT action, actor_type, target_id, change_before, change_after, actor_user_id \
         FROM audit_log WHERE organisation_id = $1",
    )
    .bind(env.organisation)
    .fetch_all(&env.pool)
    .await
    .unwrap();
    let whole: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(a::text, ''), '') FROM audit_log a WHERE organisation_id = $1",
    )
    .bind(env.organisation)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    env.forget().await;

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.get::<String, _>("action"), "deployment.iam.requested");
    assert_eq!(row.get::<String, _>("actor_type"), "user");
    assert_eq!(row.get::<Uuid, _>("target_id"), env.deployment);
    assert_eq!(
        row.get::<Option<Uuid>, _>("actor_user_id"),
        Some(env.viewer)
    );
    assert_eq!(
        row.get::<Option<Value>, _>("change_before"),
        Some(Value::Null)
    );
    assert_eq!(
        row.get::<Option<Value>, _>("change_after"),
        Some(json!({ "request_id": id, "realm": "master", "operation": "applications.list" }))
    );
    assert!(!whole.contains(TERM));
    assert!(!whole.contains("RSA-OAEP"));
    assert!(!whole.contains(&URL_SAFE_NO_PAD.encode([3u8; 96])));
}

#[tokio::test]
async fn spec_iam_9_an_unfetched_result_is_404_after_five_minutes_and_its_envelope_is_deleted() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let id = env.ask_list().await;
    env.report_done(&id, envelope()).await;
    let lifetime: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - created_at)::float8 FROM iam_requests \
         WHERE request_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE iam_requests SET expires_at = now() - interval '1 second' \
         WHERE request_id = $1::uuid",
    )
    .bind(&id)
    .execute(&env.pool)
    .await
    .unwrap();

    let (status, _) = env.poll(&env.owner_app(), &id).await;
    let purged = AutharieService::new(env.pool.clone())
        .purge_expired_iam_requests()
        .await
        .unwrap();
    let rows = env.count("iam_requests").await;
    env.forget().await;

    assert_eq!(lifetime, 300.0);
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(purged >= 1);
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn the_eleventh_outstanding_request_is_a_429_and_records_nothing() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(env.ask_list().await);
    }

    let (status, body) = env.ask(&env.owner_app(), list_body("master", None)).await;
    let recorded = (
        env.count("iam_requests").await,
        env.count("actions").await,
        env.count("audit_log").await,
    );
    env.report_done(&ids[0], envelope()).await;
    env.poll(&env.owner_app(), &ids[0]).await;
    let (freed, _) = env.ask(&env.owner_app(), list_body("master", None)).await;
    env.forget().await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["status"], 429);
    assert_eq!(recorded, (10, 10, 10));
    assert_eq!(freed, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn a_failed_request_answers_422_with_the_message_of_the_data_plane() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let id = env.ask_list().await;

    let (reported, _) = env
        .report_failed(&id, "the result is larger than 512 KiB")
        .await;
    let (first, first_body) = env.poll(&env.owner_app(), &id).await;
    let (again, _) = env.poll(&env.owner_app(), &id).await;
    env.forget().await;

    assert_eq!(reported, StatusCode::OK);
    assert_eq!(first, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(first_body["message"], "the result is larger than 512 KiB");
    assert_eq!(again, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn an_outcome_for_a_request_already_answered_is_ignored_with_a_success() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let done = env.ask_list().await;
    let failed = env.ask_list().await;
    let delivered = env.ask_list().await;

    let (first, first_body) = env.report_done(&done, envelope()).await;
    env.report_failed(&failed, "no").await;
    env.report_done(&delivered, envelope()).await;
    env.poll(&env.owner_app(), &delivered).await;

    let mut again = Vec::new();
    for id in [&done, &failed, &delivered] {
        again.push(env.report_failed(id, "again").await);
        again.push(env.report_done(id, envelope()).await);
    }
    let unknown = env
        .report_done(&Uuid::new_v4().to_string(), envelope())
        .await;
    let (still, still_body) = env.poll(&env.owner_app(), &done).await;
    let (failure, _) = env.poll(&env.owner_app(), &failed).await;
    env.forget().await;

    assert_eq!(first, StatusCode::OK);
    assert_eq!(first_body["data"]["recorded"], true);
    for (status, body) in again {
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["recorded"], false);
    }
    assert_eq!(unknown.0, StatusCode::OK);
    assert_eq!(unknown.1["data"]["recorded"], false);
    assert_eq!(still, StatusCode::OK);
    assert_eq!(still_body["data"]["envelope"], envelope());
    assert_eq!(failure, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn an_outcome_that_is_not_an_envelope_or_is_too_large_is_a_400_and_stores_nothing() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let id = env.ask_list().await;
    let mut large = envelope();
    large["ct"] = json!(URL_SAFE_NO_PAD.encode(vec![0u8; 540 * 1024]));
    let mut extra = envelope();
    extra["plain"] = json!("clear text");
    let mut refused = Vec::new();

    for body in [
        json!({ "kind": "iam", "phase": "Done", "envelope": envelope() }),
        json!({ "kind": "iam", "request_id": id, "phase": "Done" }),
        json!({ "kind": "iam", "request_id": id, "phase": "Done", "envelope": "x" }),
        json!({ "kind": "iam", "request_id": id, "phase": "Done", "envelope": large }),
        json!({ "kind": "iam", "request_id": id, "phase": "Done", "envelope": extra }),
        json!({ "kind": "iam", "request_id": id, "phase": "Exploded" }),
        json!({ "kind": "iam", "request_id": id, "phase": "Failed",
                "deployment_id": Uuid::new_v4() }),
    ] {
        refused.push(env.report(&env.herald_app(), body).await.0);
    }
    let phase: String =
        sqlx::query_scalar("SELECT phase FROM iam_requests WHERE request_id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    env.forget().await;

    assert!(
        refused
            .iter()
            .all(|status| *status == StatusCode::BAD_REQUEST)
    );
    assert_eq!(phase, "pending");
}

#[tokio::test]
async fn another_data_plane_cannot_answer_a_request_of_this_one() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let id = env.ask_list().await;

    let (status, _) = env
        .report(
            &env.app(client(&env.intruder)),
            json!({ "kind": "iam", "request_id": id, "phase": "Done", "envelope": envelope() }),
        )
        .await;
    let (anyone, _) = env
        .report(
            &env.app(user(env.owner)),
            json!({ "kind": "iam", "request_id": id, "phase": "Done", "envelope": envelope() }),
        )
        .await;
    let phase: String =
        sqlx::query_scalar("SELECT phase FROM iam_requests WHERE request_id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    env.forget().await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(anyone, StatusCode::FORBIDDEN);
    assert_eq!(phase, "pending");
}

#[tokio::test]
async fn only_the_member_who_asked_can_collect_the_answer() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    let id = env.ask_list().await;
    env.report_done(&id, envelope()).await;

    let (other, _) = env.poll(&env.viewer_app(), &id).await;
    let (asker, body) = env.poll(&env.owner_app(), &id).await;
    env.forget().await;

    assert_eq!(other, StatusCode::NOT_FOUND);
    assert_eq!(asker, StatusCode::OK);
    assert_eq!(body["data"]["envelope"], envelope());
}

#[tokio::test]
async fn a_request_nobody_asked_is_a_404() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;

    let (status, _) = env
        .poll(&env.owner_app(), &Uuid::new_v4().to_string())
        .await;
    env.forget().await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_search_term_leaves_the_stored_action_once_published_and_never_shows_in_the_listing() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey").await;
    env.ask(&env.owner_app(), list_body("master", Some(TERM)))
        .await;

    let stored = env.iam_actions().await;
    assert_eq!(stored[0]["operation"]["search"], TERM);

    let (status, claimed) = call(
        &env.herald_app(),
        Method::POST,
        &format!("/dataplanes/{}/actions:claim", env.dataplane),
        Some(json!({ "deployment_ids": [env.deployment], "max": 10, "lease_seconds": 60 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let claimed = claimed["data"].as_array().unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        claimed[0]["payload"]["data"]["operation"]["search"], TERM,
        "the data plane gets the search term on the claim"
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

    let stored = env.iam_actions().await;
    let (listed_status, listed) = call(&env.owner_app(), Method::GET, &env.actions(), None).await;
    env.forget().await;

    assert_eq!(
        stored[0]["operation"],
        json!({ "type": "applications.list" })
    );
    assert_eq!(stored[0]["realm"], "master");
    assert!(!stored[0].to_string().contains(TERM));
    assert_eq!(listed_status, StatusCode::OK);
    assert!(!listed.to_string().contains(TERM));
}
