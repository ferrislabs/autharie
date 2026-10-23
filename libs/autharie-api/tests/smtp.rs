use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::{
        actions::{get_action::get_action_handler, list_actions::list_actions_handler},
        dataplanes::{
            ack_actions::ack_actions_handler, claim_actions::claim_actions_handler,
            report_outcome::report_outcome_handler,
        },
        deployments::{
            smtp::{
                delete_smtp_handler, get_smtp_handler, list_branding_realms_handler,
                put_smtp_handler,
            },
            webhooks::list_webhook_realms_handler,
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
const PASSWORD: &str = "smtp-canary-0123456789";
const OTHER_PASSWORD: &str = "smtp-canary-9876543210";
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
            .typed_get(get_smtp_handler)
            .typed_put(put_smtp_handler)
            .typed_delete(delete_smtp_handler)
            .typed_get(list_branding_realms_handler)
            .typed_get(list_webhook_realms_handler)
            .typed_get(list_actions_handler)
            .typed_get(get_action_handler)
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
            act: None,
        }))
    }

    fn smtp(&self, realm: &str) -> String {
        format!(
            "/organisations/{}/deployments/{}/branding/{realm}/smtp",
            self.organisation, self.deployment
        )
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
}

fn user(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "wh".to_string(),
        email: None,
        name: None,
        roles: vec![],
        scopes: vec![],
        act: None,
    })
}

fn smtp_body(password: Option<&str>) -> Value {
    let mut body = json!({
        "host": "smtp.example.com",
        "port": 587,
        "encryption": "starttls",
        "username": "apikey",
        "from_email": "no-reply@example.com",
        "from_name": "Acme"
    });
    if let Some(password) = password {
        body["password"] = json!(password);
    }
    body
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

impl Env {
    async fn save(&self, realm: &str, password: Option<&str>) -> (StatusCode, Value) {
        call(
            &self.owner_app(),
            Method::PUT,
            &self.smtp(realm),
            Some(smtp_body(password)),
        )
        .await
    }

    async fn publish_everything(&self) {
        let (status, claimed) = call(
            &self.herald_app(),
            Method::POST,
            &format!("/dataplanes/{}/actions:claim", self.dataplane),
            Some(json!({ "deployment_ids": [self.deployment], "max": 10, "lease_seconds": 60 })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{claimed}");

        let published: Vec<String> = claimed["data"]
            .as_array()
            .expect("claimed actions")
            .iter()
            .map(|action| action["id"].as_str().unwrap().to_string())
            .collect();
        let (status, acked) = call(
            &self.herald_app(),
            Method::POST,
            &format!(
                "/dataplanes/{}/deployments/{}/actions:ack",
                self.dataplane, self.deployment
            ),
            Some(json!({ "published": published, "failed": [] })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{acked}");
    }

    async fn report_smtp(&self, revision: u64, realm: &str, phase: &str, message: Option<&str>) {
        let mut item = json!({ "phase": phase });
        if let Some(message) = message {
            item["message"] = json!(message);
        }
        let status = self
            .report(json!({
                "deployment_id": self.deployment,
                "kind": "smtp",
                "revision": revision,
                "realm": realm,
                "items": [item]
            }))
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    async fn sealed_password(&self, realm: &str) -> Vec<u8> {
        sqlx::query_scalar(
            "SELECT password_ciphertext FROM deployment_smtp_configs \
             WHERE deployment_id = $1 AND realm = $2",
        )
        .bind(self.deployment)
        .bind(realm)
        .fetch_one(&self.pool)
        .await
        .expect("a sealed password")
    }

    async fn phase(&self, realm: &str) -> Value {
        let (status, shown) = call(&self.owner_app(), Method::GET, &self.smtp(realm), None).await;
        assert_eq!(status, StatusCode::OK, "{shown}");

        shown["data"]["state"].clone()
    }
}

#[tokio::test]
async fn the_first_save_without_a_password_is_a_400_spec_br_3() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let (status, body) = env.save("master", None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(env.actions_of_type("deployment.smtp").await.is_empty());
    let (_, shown) = call(&env.owner_app(), Method::GET, &env.smtp("master"), None).await;
    assert_eq!(shown, json!({ "data": null }));

    env.forget().await;
}

#[tokio::test]
async fn a_save_is_accepted_pending_and_the_action_carries_the_password_spec_br_3() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let (status, saved) = env.save("master", Some(PASSWORD)).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{saved}");
    let view = &saved["data"];
    assert_eq!(view["host"], "smtp.example.com");
    assert_eq!(view["port"], 587);
    assert_eq!(view["encryption"], "starttls");
    assert_eq!(view["username"], "apikey");
    assert_eq!(view["from_email"], "no-reply@example.com");
    assert_eq!(view["from_name"], "Acme");
    assert_eq!(view["password_set"], true);
    assert_eq!(view["state"], json!({ "phase": "Pending" }));
    assert!(view["updated_at"].is_string());

    let sent = env.actions_of_type("deployment.smtp").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0],
        json!({
            "deployment_id": env.deployment,
            "namespace": format!("wh-{}", env.organisation),
            "revision": 1,
            "realm": "master",
            "smtp": {
                "host": "smtp.example.com",
                "port": 587,
                "encryption": "starttls",
                "username": "apikey",
                "from_email": "no-reply@example.com",
                "from_name": "Acme",
                "password": PASSWORD
            }
        })
    );

    let (status, shown) = call(&env.owner_app(), Method::GET, &env.smtp("master"), None).await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown["data"]["password_set"], true);
    assert!(!saved.to_string().contains(PASSWORD));
    assert!(!shown.to_string().contains(PASSWORD));
    assert!(shown["data"].get("password").is_none());

    let sealed = env.sealed_password("master").await;
    assert!(!String::from_utf8_lossy(&sealed).contains(PASSWORD));

    env.forget().await;
}

#[tokio::test]
async fn saving_again_without_a_password_keeps_the_stored_one_spec_br_4() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;
    let before = env.sealed_password("master").await;

    let mut change = smtp_body(None);
    change["from_name"] = json!("Acme Support");
    let (status, saved) = call(
        &env.owner_app(),
        Method::PUT,
        &env.smtp("master"),
        Some(change),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{saved}");
    assert_eq!(saved["data"]["from_name"], "Acme Support");
    assert_eq!(saved["data"]["password_set"], true);
    assert_eq!(env.sealed_password("master").await, before);
    let sent = env.actions_of_type("deployment.smtp").await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["revision"], 2);
    assert!(sent[1]["smtp"]["password"].is_null());
    assert!(!sent[1].to_string().contains(PASSWORD));

    let (status, _) = env.save("master", Some(OTHER_PASSWORD)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_ne!(env.sealed_password("master").await, before);
    assert_eq!(
        env.actions_of_type("deployment.smtp").await[2]["smtp"]["password"],
        OTHER_PASSWORD
    );

    env.forget().await;
}

#[tokio::test]
async fn the_password_leaves_the_stored_action_once_published_and_never_shows_in_the_api_spec_br_5()
{
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;
    env.save("master", Some(OTHER_PASSWORD)).await;

    let (status, listed) = call(&env.owner_app(), Method::GET, &env.actions(), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(!listed.to_string().contains(PASSWORD));
    assert!(!listed.to_string().contains(OTHER_PASSWORD));
    let first = listed["data"][0]["id"]
        .as_str()
        .expect("an action id")
        .to_string();
    let (status, one) = call(
        &env.owner_app(),
        Method::GET,
        &format!("{}/{first}", env.actions()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert!(!one.to_string().contains(PASSWORD));
    assert!(!one.to_string().contains(OTHER_PASSWORD));
    assert_eq!(one["data"]["payload"]["data"]["realm"], "master");

    let stored = env.actions_of_type("deployment.smtp").await;
    assert_eq!(stored[0]["smtp"]["password"], PASSWORD);

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
            .any(|action| action["payload"]["data"]["smtp"]["password"] == PASSWORD),
        "the data plane gets the password on the claim"
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

    let stored = env.actions_of_type("deployment.smtp").await;
    assert_eq!(stored.len(), 2);
    for payload in stored {
        assert!(payload["smtp"].get("password").is_none(), "{payload}");
        assert!(!payload.to_string().contains(PASSWORD));
        assert!(!payload.to_string().contains(OTHER_PASSWORD));
        assert_eq!(payload["smtp"]["host"], "smtp.example.com");
    }

    env.forget().await;
}

#[tokio::test]
async fn the_password_leaves_the_stored_action_when_it_fails_to_publish() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;

    let (_, claimed) = call(
        &env.herald_app(),
        Method::POST,
        &format!("/dataplanes/{}/actions:claim", env.dataplane),
        Some(json!({ "deployment_ids": [env.deployment], "max": 10, "lease_seconds": 60 })),
    )
    .await;
    let id = claimed["data"][0]["id"]
        .as_str()
        .expect("an action")
        .to_string();
    let (status, acked) = call(
        &env.herald_app(),
        Method::POST,
        &format!(
            "/dataplanes/{}/deployments/{}/actions:ack",
            env.dataplane, env.deployment
        ),
        Some(json!({
            "published": [],
            "failed": [{ "action_id": id, "reason": "PublishFailed" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{acked}");

    let stored = env.actions_of_type("deployment.smtp").await;
    assert!(stored[0]["smtp"].get("password").is_none(), "{}", stored[0]);

    env.forget().await;
}

#[tokio::test]
async fn an_outcome_sets_applied_or_failed_and_an_older_one_changes_nothing_spec_br_11() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;
    env.save("master", None).await;
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));

    env.report_smtp(1, "master", "Applied", None).await;
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));

    env.report_smtp(2, "master", "Failed", Some("535 authentication failed"))
        .await;
    assert_eq!(
        env.phase("master").await,
        json!({ "phase": "Failed", "message": "535 authentication failed" })
    );

    env.report_smtp(2, "master", "Applied", None).await;
    assert_eq!(env.phase("master").await, json!({ "phase": "Applied" }));

    env.report_smtp(1, "master", "Failed", Some("old news"))
        .await;
    assert_eq!(env.phase("master").await, json!({ "phase": "Applied" }));

    env.forget().await;
}

#[tokio::test]
async fn an_outcome_is_answered_recorded_only_when_it_changed_something() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;

    let report = |revision: u64, phase: &str| {
        let env = &env;
        let body = json!({
            "kind": "smtp", "revision": revision, "realm": "master",
            "items": [{ "phase": phase }]
        });
        async move {
            call(&env.herald_app(), Method::POST, &env.outcome(), Some(body))
                .await
                .1
        }
    };

    assert_eq!(report(1, "Applied").await["data"]["recorded"], true);
    assert_eq!(report(1, "Applied").await["data"]["recorded"], false);
    assert_eq!(report(0, "Failed").await["data"]["recorded"], false);

    env.forget().await;
}

#[tokio::test]
async fn a_malformed_smtp_report_is_a_400_and_a_stranger_is_a_403() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;

    for body in [
        json!({ "kind": "smtp" }),
        json!({ "kind": "smtp", "revision": 1, "realm": "master" }),
        json!({ "kind": "smtp", "revision": 1, "realm": "master", "items": [{ "phase": "Exploded" }] }),
    ] {
        let (status, answer) =
            call(&env.herald_app(), Method::POST, &env.outcome(), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    }

    let stranger = Identity::Client(Client {
        id: "herald-someone-else".to_string(),
        client_id: "herald".to_string(),
        roles: vec![],
        scopes: vec![],
        act: None,
    });
    let (status, body) = call(
        &env.app(stranger),
        Method::POST,
        &env.outcome(),
        Some(json!({
            "kind": "smtp", "revision": 1, "realm": "master",
            "items": [{ "phase": "Applied" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));

    env.forget().await;
}

#[tokio::test]
async fn the_revision_keeps_growing_across_a_removal_and_a_new_save_spec_br_11() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;

    let (status, removed) = call(&env.owner_app(), Method::DELETE, &env.smtp("master"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{removed}");
    let (_, shown) = call(&env.owner_app(), Method::GET, &env.smtp("master"), None).await;
    assert_eq!(shown, json!({ "data": null }));

    let (status, again) = env.save("master", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{again}");
    let (status, _) = env.save("master", Some(PASSWORD)).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let sent = env.actions_of_type("deployment.smtp").await;
    let revisions: Vec<_> = sent
        .iter()
        .map(|payload| payload["revision"].clone())
        .collect();
    assert_eq!(revisions, vec![json!(1), json!(2), json!(3)]);
    assert!(sent[1]["smtp"].is_null());
    assert_eq!(sent[1]["realm"], "master");

    env.report_smtp(2, "master", "Applied", None).await;
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));
    env.report_smtp(3, "master", "Applied", None).await;
    assert_eq!(env.phase("master").await, json!({ "phase": "Applied" }));

    env.forget().await;
}

#[tokio::test]
async fn a_removal_outcome_for_a_deleted_config_is_a_no_op() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;
    call(&env.owner_app(), Method::DELETE, &env.smtp("master"), None).await;

    env.report_smtp(2, "master", "Applied", None).await;

    let (_, shown) = call(&env.owner_app(), Method::GET, &env.smtp("master"), None).await;
    assert_eq!(shown, json!({ "data": null }));

    env.forget().await;
}

#[tokio::test]
async fn removing_what_is_not_there_is_a_404() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    let (status, body) = call(&env.owner_app(), Method::DELETE, &env.smtp("master"), None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(env.actions_of_type("deployment.smtp").await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn a_viewer_reads_but_cannot_change_spec_br_12() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;

    let (status, shown) = call(&env.viewer_app(), Method::GET, &env.smtp("master"), None).await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown["data"]["password_set"], true);
    assert!(!shown.to_string().contains(PASSWORD));

    let (status, realms) = call(
        &env.viewer_app(),
        Method::GET,
        &format!(
            "/organisations/{}/deployments/{}/branding/realms",
            env.organisation, env.deployment
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{realms}");

    let (status, body) = call(
        &env.viewer_app(),
        Method::PUT,
        &env.smtp("master"),
        Some(smtp_body(Some(PASSWORD))),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = call(&env.viewer_app(), Method::DELETE, &env.smtp("master"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    assert_eq!(env.actions_of_type("deployment.smtp").await.len(), 1);
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));

    env.forget().await;
}

#[tokio::test]
async fn a_stranger_to_the_organisation_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let outsider = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'out', $3)")
        .bind(outsider)
        .bind(format!("{outsider}@smtp-api.test"))
        .bind(outsider.to_string())
        .execute(&env.pool)
        .await
        .expect("a user");

    let (status, body) = call(
        &env.app(user(outsider)),
        Method::GET,
        &env.smtp("master"),
        None,
    )
    .await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "{status}: {body}"
    );
    assert!(!body.to_string().contains("smtp.example.com"));

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(outsider)
        .execute(&env.pool)
        .await
        .expect("cleanup");
    env.forget().await;
}

#[tokio::test]
async fn a_deployment_that_is_not_ferriskey_is_refused_spec_br_10() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "keycloak").await;

    let (status, body) = env.save("master", Some(PASSWORD)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, body) = call(&env.owner_app(), Method::DELETE, &env.smtp("master"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, shown) = call(&env.owner_app(), Method::GET, &env.smtp("master"), None).await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown, json!({ "data": null }));
    assert!(env.actions_of_type("deployment.smtp").await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn an_invalid_host_port_address_or_realm_is_a_400() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;

    for (field, value) in [
        ("host", json!("smtp://mail.example.com")),
        ("host", json!("mail example.com")),
        ("host", json!("mail.example.com/relay")),
        ("host", json!("")),
        ("port", json!(0)),
        ("port", json!(65536)),
        ("port", json!(-5)),
        ("from_email", json!("nobody")),
        ("from_email", json!("a@b@c.com")),
        ("username", json!("")),
        ("from_name", json!("n".repeat(256))),
        ("password", json!("")),
    ] {
        let mut body = smtp_body(Some(PASSWORD));
        body[field] = value.clone();

        let (status, answer) = call(
            &env.owner_app(),
            Method::PUT,
            &env.smtp("master"),
            Some(body),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}={value}: {answer}");
        assert!(!answer.to_string().contains(PASSWORD));
    }

    let (status, answer) = env.save("not%20a%20realm", Some(PASSWORD)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert!(env.actions_of_type("deployment.smtp").await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn the_realms_are_the_webhooks_realms_and_a_provider_is_set_per_realm() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    let branding_realms = format!(
        "/organisations/{}/deployments/{}/branding/realms",
        env.organisation, env.deployment
    );
    let webhook_realms = format!(
        "/organisations/{}/deployments/{}/webhook-realms",
        env.organisation, env.deployment
    );

    let (status, realms) = call(&env.owner_app(), Method::GET, &branding_realms, None).await;
    assert_eq!(status, StatusCode::OK, "{realms}");
    assert_eq!(realms, json!({ "data": ["master"] }));

    let (status, answer) = env.save("customers", Some(PASSWORD)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");

    let reported = env
        .report(json!({
            "kind": "webhooks",
            "revision": 1,
            "webhooks": [],
            "realms": ["master", "customers"]
        }))
        .await;
    assert_eq!(reported, StatusCode::OK);

    let (_, realms) = call(&env.owner_app(), Method::GET, &branding_realms, None).await;
    let (_, webhooks) = call(&env.owner_app(), Method::GET, &webhook_realms, None).await;
    assert_eq!(realms, json!({ "data": ["master", "customers"] }));
    assert_eq!(webhooks, realms);

    let (status, _) = env.save("customers", Some(PASSWORD)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    env.save("master", Some(OTHER_PASSWORD)).await;
    let sent = env.actions_of_type("deployment.smtp").await;
    assert_eq!(sent[0]["realm"], "customers");
    assert_eq!(sent[0]["revision"], 1);
    assert_eq!(sent[1]["realm"], "master");
    assert_eq!(sent[1]["revision"], 1);

    env.report_smtp(1, "customers", "Applied", None).await;
    assert_eq!(env.phase("customers").await, json!({ "phase": "Applied" }));
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));

    env.forget().await;
}

#[tokio::test]
async fn publishing_leaves_the_state_and_the_sealed_password_alone() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "starter", "ferriskey").await;
    env.save("master", Some(PASSWORD)).await;
    let before = env.sealed_password("master").await;

    env.publish_everything().await;

    assert_eq!(env.sealed_password("master").await, before);
    assert_eq!(env.phase("master").await, json!({ "phase": "Pending" }));

    env.forget().await;
}
