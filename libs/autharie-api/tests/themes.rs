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
            smtp::list_branding_realms_handler,
            themes::{
                activate_theme_handler, create_theme_handler, deactivate_themes_handler,
                delete_theme_handler, list_themes_handler, update_theme_handler,
            },
            webhooks::list_webhook_realms_handler,
        },
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
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

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

struct Env {
    pool: PgPool,
    organisation: Uuid,
    owner: Uuid,
    viewer: Uuid,
    dataplane: Uuid,
    deployment: Uuid,
    herald: String,
}

impl Env {
    async fn new(pool: PgPool, kind: &str, features: Option<&[&str]>) -> Self {
        let env = Self {
            pool,
            organisation: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            viewer: Uuid::new_v4(),
            dataplane: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            herald: format!("herald-{}", Uuid::new_v4()),
        };
        let tag = format!("th-{}", env.organisation);

        for user in [env.owner, env.viewer] {
            sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'th', $3)")
                .bind(user)
                .bind(format!("{user}@themes-api.test"))
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
        .bind("starter")
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
        if let Some(features) = features {
            env.subscribe(features).await;
        }
        env
    }

    async fn subscribe(&self, features: &[&str]) {
        let features: Vec<String> = features.iter().map(|feature| feature.to_string()).collect();
        sqlx::query(
            "INSERT INTO billing_subscriptions \
             (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, \
              features, starts_at, ends_at) \
             VALUES ($1, $2, $3, 'legacy-standard', 4900, 'standard', $4, \
                     now() - interval '400 days', now() - interval '399 days')",
        )
        .bind(Uuid::new_v4())
        .bind(self.organisation)
        .bind(self.deployment)
        .bind(&features)
        .execute(&self.pool)
        .await
        .expect("a subscription");
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
            (
                "DELETE FROM billing_subscriptions WHERE deployment_id = $1",
                self.deployment,
            ),
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
            service: AutharieService::new(self.pool.clone()),
            certificate_source: None,
            quickwit_search: None,
            quickwit_traces: None,
            billing: None,
        }
    }

    fn app(&self, identity: Identity) -> Router {
        Router::new()
            .typed_get(list_themes_handler)
            .typed_post(create_theme_handler)
            .typed_put(update_theme_handler)
            .typed_delete(delete_theme_handler)
            .typed_post(activate_theme_handler)
            .typed_post(deactivate_themes_handler)
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

    fn themes(&self, realm: &str) -> String {
        format!(
            "/organisations/{}/deployments/{}/branding/{realm}/themes",
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
        username: "th".to_string(),
        email: None,
        name: None,
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
const BRANDING: Option<&[&str]> = Some(&["branding", "mfa"]);

fn tokens(primary: &str) -> Value {
    json!({
        "colors": { "primaryButton": primary },
        "borders": { "buttonRadius": 8, "widgetShadow": "small" }
    })
}

impl Env {
    async fn create(&self, name: &str, tokens: Value) -> (StatusCode, Value) {
        call(
            &self.owner_app(),
            Method::POST,
            &self.themes("master"),
            Some(json!({ "name": name, "tokens": tokens })),
        )
        .await
    }

    async fn made(&self, name: &str, primary: &str) -> String {
        let (status, created) = self.create(name, tokens(primary)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");

        created["data"]["id"].as_str().expect("an id").to_string()
    }

    async fn update(&self, id: &str, body: Value) -> (StatusCode, Value) {
        call(
            &self.owner_app(),
            Method::PUT,
            &format!("{}/{id}", self.themes("master")),
            Some(body),
        )
        .await
    }

    async fn remove(&self, id: &str) -> StatusCode {
        call(
            &self.owner_app(),
            Method::DELETE,
            &format!("{}/{id}", self.themes("master")),
            None,
        )
        .await
        .0
    }

    async fn activate(&self, id: &str) -> (StatusCode, Value) {
        call(
            &self.owner_app(),
            Method::POST,
            &format!("{}/{id}/activate", self.themes("master")),
            None,
        )
        .await
    }

    async fn deactivate(&self) -> (StatusCode, Value) {
        call(
            &self.owner_app(),
            Method::POST,
            &format!("{}/deactivate", self.themes("master")),
            None,
        )
        .await
    }

    async fn listed(&self) -> Value {
        let (status, listed) =
            call(&self.owner_app(), Method::GET, &self.themes("master"), None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");

        listed
    }

    async fn sent(&self) -> Vec<Value> {
        self.actions_of_type("deployment.portal_theme").await
    }

    async fn report_theme(&self, revision: u64, realm: &str, phase: &str, message: Option<&str>) {
        let mut item = json!({ "phase": phase });
        if let Some(message) = message {
            item["message"] = json!(message);
        }
        let status = self
            .report(json!({
                "deployment_id": self.deployment,
                "kind": "portal_theme",
                "revision": revision,
                "realm": realm,
                "items": [item]
            }))
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    async fn sync_state(&self) -> Value {
        self.listed().await["state"].clone()
    }
}

#[tokio::test]
async fn a_new_theme_is_listed_inactive_and_sends_nothing_spec_br_6() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;

    let (status, created) = env.create("Autumn", tokens("#112233")).await;

    assert_eq!(status, StatusCode::CREATED, "{created}");
    let view = &created["data"];
    assert_eq!(view["name"], "Autumn");
    assert_eq!(view["active"], false);
    assert_eq!(view["tokens"], tokens("#112233"));
    assert!(view["id"].is_string());
    assert!(view["created_at"].is_string());
    assert!(view["updated_at"].is_string());

    let listed = env.listed().await;
    assert_eq!(listed["data"], json!([view]));
    assert_eq!(listed["state"], Value::Null);
    assert!(env.sent().await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn activating_sends_the_tokens_and_the_state_is_pending_until_the_data_plane_reports_spec_br_6()
 {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;

    let (status, activated) = env.activate(&id).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{activated}");
    assert_eq!(activated["data"]["id"], id);
    assert_eq!(activated["data"]["active"], true);
    assert_eq!(
        env.sent().await,
        vec![json!({
            "deployment_id": env.deployment,
            "namespace": format!("th-{}", env.organisation),
            "revision": 1,
            "realm": "master",
            "config": tokens("#112233")
        })]
    );
    let listed = env.listed().await;
    assert_eq!(listed["data"][0]["active"], true);
    assert_eq!(
        listed["state"],
        json!({ "phase": "Pending", "revision": 1 })
    );

    env.report_theme(1, "master", "Applied", None).await;
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Applied", "revision": 1 })
    );

    env.forget().await;
}

#[tokio::test]
async fn activating_another_theme_makes_it_the_only_active_one_with_a_higher_revision() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let first = env.made("Autumn", "#112233").await;
    let second = env.made("Winter", "#445566").await;
    env.activate(&first).await;

    let (status, _) = env.activate(&second).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let listed = env.listed().await;
    let active: Vec<&Value> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|theme| theme["active"] == true)
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["id"], second);
    let sent = env.sent().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["revision"], 2);
    assert_eq!(sent[1]["config"], tokens("#445566"));
    assert_eq!(
        listed["state"],
        json!({ "phase": "Pending", "revision": 2 })
    );

    env.forget().await;
}

#[tokio::test]
async fn using_ferriskeys_default_sends_empty_tokens_spec_br_15() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;
    env.activate(&id).await;

    let (status, answer) = env.deactivate().await;

    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    assert_eq!(answer, json!({}));
    let sent = env.sent().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["config"], json!({}));
    assert_eq!(sent[1]["revision"], 2);
    let listed = env.listed().await;
    assert_eq!(listed["data"][0]["active"], false);
    assert_eq!(
        listed["state"],
        json!({ "phase": "Pending", "revision": 2 })
    );

    env.forget().await;
}

#[tokio::test]
async fn saving_the_active_theme_sends_it_and_saving_another_sends_nothing_spec_br_7() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let active = env.made("Autumn", "#112233").await;
    let other = env.made("Winter", "#445566").await;
    env.activate(&active).await;

    let (status, saved) = env
        .update(&other, json!({ "tokens": tokens("#abcdef") }))
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["tokens"], tokens("#abcdef"));
    assert_eq!(saved["data"]["active"], false);
    assert_eq!(
        env.sent().await.len(),
        1,
        "a theme that is not active sends nothing"
    );

    let (status, renamed) = env.update(&active, json!({ "name": "Fall" })).await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["data"]["name"], "Fall");
    assert_eq!(env.sent().await.len(), 1, "a rename sends nothing");

    let (status, saved) = env
        .update(
            &active,
            json!({ "name": "Fall", "tokens": tokens("#778899") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["active"], true);
    let sent = env.sent().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["revision"], 2);
    assert_eq!(sent[1]["config"], tokens("#778899"));

    env.forget().await;
}

#[tokio::test]
async fn the_active_theme_cannot_be_deleted_but_the_others_can_spec_br_7() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let active = env.made("Autumn", "#112233").await;
    let other = env.made("Winter", "#445566").await;
    env.activate(&active).await;

    assert_eq!(env.remove(&active).await, StatusCode::CONFLICT);
    assert_eq!(env.listed().await["data"].as_array().unwrap().len(), 2);

    assert_eq!(env.remove(&other).await, StatusCode::NO_CONTENT);
    assert_eq!(env.listed().await["data"].as_array().unwrap().len(), 1);
    assert_eq!(env.sent().await.len(), 1, "deleting a theme sends nothing");

    env.deactivate().await;
    assert_eq!(env.remove(&active).await, StatusCode::NO_CONTENT);
    assert_eq!(env.listed().await["data"], json!([]));

    env.forget().await;
}

#[tokio::test]
async fn a_realm_cannot_hold_an_eleventh_theme_spec_br_7() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    for index in 0..10 {
        env.made(&format!("Theme {index}"), "#112233").await;
    }

    let (status, refused) = env.create("One too many", tokens("#112233")).await;

    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(env.listed().await["data"].as_array().unwrap().len(), 10);

    env.forget().await;
}

#[tokio::test]
async fn a_name_is_unique_in_a_realm_whatever_its_case() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    env.made("Autumn", "#112233").await;
    let winter = env.made("Winter", "#445566").await;

    let (status, duplicate) = env.create("aUTUMN", tokens("#112233")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");

    let (status, rename) = env.update(&winter, json!({ "name": "AUTUMN" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{rename}");

    let (status, kept) = env.update(&winter, json!({ "name": "WINTER" })).await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(env.listed().await["data"].as_array().unwrap().len(), 2);

    env.forget().await;
}

#[tokio::test]
async fn an_invalid_token_is_a_400_naming_the_field_spec_br_8() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;

    for (bad, field) in [
        (
            json!({ "colors": { "primaryButton": "#12" } }),
            "colors.primaryButton",
        ),
        (
            json!({ "colors": { "links": "#12345678" } }),
            "colors.links",
        ),
        (
            json!({ "borders": { "buttonRadius": 65 } }),
            "borders.buttonRadius",
        ),
        (
            json!({ "borders": { "widgetShadow": "huge" } }),
            "borders.widgetShadow",
        ),
        (
            json!({ "spacing": { "fieldGap": 129 } }),
            "spacing.fieldGap",
        ),
        (json!({ "fonts": { "baseSize": 9 } }), "fonts.baseSize"),
        (
            json!({ "fonts": { "title": { "weight": 650, "sizePct": 100 } } }),
            "fonts.title.weight",
        ),
        (
            json!({ "fonts": { "url": "http://fonts.example.com/a.css" } }),
            "fonts.url",
        ),
        (
            json!({ "colors": { "primary": "#112233" } }),
            "colors.primary",
        ),
        (json!({ "page": {} }), "page"),
    ] {
        let (status, created) = env.create("Other", bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {created}");
        assert!(
            created["message"]
                .to_string()
                .contains(&format!("{field}:")),
            "{field}: {created}"
        );

        let (status, saved) = env.update(&id, json!({ "tokens": bad })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{saved}");
    }

    let too_big = json!({ "notes": "x".repeat(40_000) });
    let (status, refused) = env.create("Big", too_big).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    for body in [
        json!({ "name": "", "tokens": {} }),
        json!({ "name": "n".repeat(81), "tokens": {} }),
        json!({ "name": "Ok", "tokens": [] }),
        json!({ "name": "Ok", "tokens": null }),
    ] {
        let (status, refused) = call(
            &env.owner_app(),
            Method::POST,
            &env.themes("master"),
            Some(body.clone()),
        )
        .await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
            "{body}: {refused}"
        );
    }
    let (status, _) = env.update(&id, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(env.listed().await["data"].as_array().unwrap().len(), 1);
    assert!(env.sent().await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn a_bad_realm_is_a_400_and_a_realm_the_deployment_lacks_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;

    let (status, refused) = call(
        &env.owner_app(),
        Method::GET,
        &env.themes("bad.realm"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    let (status, refused) = call(
        &env.owner_app(),
        Method::POST,
        &env.themes("nowhere"),
        Some(json!({ "name": "Autumn", "tokens": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    env.forget().await;
}

#[tokio::test]
async fn an_unknown_theme_is_a_404() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let missing = Uuid::new_v4().to_string();

    assert_eq!(
        env.update(&missing, json!({ "name": "x" })).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(env.remove(&missing).await, StatusCode::NOT_FOUND);
    assert_eq!(env.activate(&missing).await.0, StatusCode::NOT_FOUND);
    assert!(env.sent().await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn a_viewer_sees_everything_and_changes_nothing_spec_br_12() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;
    env.activate(&id).await;
    let before = env.listed().await;
    let viewer = env.viewer_app();
    let themes = env.themes("master");

    let (status, seen) = call(&viewer, Method::GET, &themes, None).await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen, before);

    for (method, uri, body) in [
        (
            Method::POST,
            themes.clone(),
            Some(json!({ "name": "Other", "tokens": {} })),
        ),
        (
            Method::PUT,
            format!("{themes}/{id}"),
            Some(json!({ "name": "Other" })),
        ),
        (Method::DELETE, format!("{themes}/{id}"), None),
        (Method::POST, format!("{themes}/{id}/activate"), None),
        (Method::POST, format!("{themes}/deactivate"), None),
    ] {
        let (status, refused) = call(&viewer, method.clone(), &uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {refused}");
    }

    assert_eq!(env.listed().await, before);
    assert_eq!(env.sent().await.len(), 1);

    env.forget().await;
}

#[tokio::test]
async fn a_deployment_without_the_branding_feature_lists_but_cannot_change_spec_br_13() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", Some(&["mfa"])).await;
    let id = Uuid::new_v4();

    let (status, listed) = call(&env.owner_app(), Method::GET, &env.themes("master"), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed, json!({ "data": [], "state": null }));

    let themes = env.themes("master");
    for (method, uri, body) in [
        (
            Method::POST,
            themes.clone(),
            Some(json!({ "name": "Autumn", "tokens": {} })),
        ),
        (
            Method::PUT,
            format!("{themes}/{id}"),
            Some(json!({ "name": "Other" })),
        ),
        (Method::DELETE, format!("{themes}/{id}"), None),
        (Method::POST, format!("{themes}/{id}/activate"), None),
        (Method::POST, format!("{themes}/deactivate"), None),
    ] {
        let (status, refused) = call(&env.owner_app(), method.clone(), &uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {refused}");
    }
    assert!(env.sent().await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn a_deployment_with_no_subscription_cannot_change_either() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", None).await;

    let (status, refused) = env.create("Autumn", tokens("#112233")).await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

    env.forget().await;
}

#[tokio::test]
async fn another_kind_of_deployment_answers_empty_and_refuses_changes() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "keycloak", BRANDING).await;

    let (status, listed) = call(&env.owner_app(), Method::GET, &env.themes("master"), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed, json!({ "data": [], "state": null }));

    let (status, refused) = env.create("Autumn", tokens("#112233")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(env.deactivate().await.0, StatusCode::CONFLICT);
    assert_eq!(
        env.activate(&Uuid::new_v4().to_string()).await.0,
        StatusCode::CONFLICT
    );
    assert!(env.sent().await.is_empty());

    env.forget().await;
}

#[tokio::test]
async fn a_caller_outside_the_organisation_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;

    let (status, _) = call(
        &env.owner_app(),
        Method::GET,
        &format!(
            "/organisations/{}/deployments/{}/branding/master/themes",
            Uuid::new_v4(),
            env.deployment
        ),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);

    env.forget().await;
}

#[tokio::test]
async fn an_outcome_sets_applied_or_failed_and_an_older_one_changes_nothing_spec_br_11() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;
    env.activate(&id).await;
    env.deactivate().await;
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Pending", "revision": 2 })
    );

    env.report_theme(1, "master", "Applied", None).await;
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Pending", "revision": 2 })
    );

    env.report_theme(2, "master", "Failed", Some("404 portal themes"))
        .await;
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Failed", "message": "404 portal themes", "revision": 2 })
    );

    env.report_theme(2, "master", "Applied", None).await;
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Applied", "revision": 2 })
    );

    env.report_theme(1, "master", "Failed", Some("old news"))
        .await;
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Applied", "revision": 2 })
    );

    env.forget().await;
}

#[tokio::test]
async fn an_outcome_is_answered_recorded_only_when_it_changed_something() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;
    env.activate(&id).await;

    let report = |revision: u64, realm: &str, phase: &str| {
        let env = &env;
        let body = json!({
            "kind": "portal_theme", "revision": revision, "realm": realm,
            "items": [{ "phase": phase }]
        });
        async move {
            call(&env.herald_app(), Method::POST, &env.outcome(), Some(body))
                .await
                .1
        }
    };

    assert_eq!(
        report(1, "master", "Applied").await["data"]["recorded"],
        true
    );
    assert_eq!(
        report(1, "master", "Applied").await["data"]["recorded"],
        false
    );
    assert_eq!(
        report(0, "master", "Failed").await["data"]["recorded"],
        false
    );
    assert_eq!(
        report(1, "customers", "Applied").await["data"]["recorded"],
        false
    );

    env.forget().await;
}

#[tokio::test]
async fn the_revision_never_goes_back_even_when_every_theme_is_deleted_spec_br_11() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let first = env.made("Autumn", "#112233").await;
    env.activate(&first).await;
    env.deactivate().await;
    env.remove(&first).await;
    let second = env.made("Autumn", "#445566").await;
    env.activate(&second).await;

    let revisions: Vec<u64> = env
        .sent()
        .await
        .iter()
        .map(|payload| payload["revision"].as_u64().unwrap())
        .collect();

    assert_eq!(revisions, vec![1, 2, 3]);
    assert_eq!(
        env.sync_state().await,
        json!({ "phase": "Pending", "revision": 3 })
    );

    env.forget().await;
}

#[tokio::test]
async fn a_malformed_portal_theme_report_is_a_400_and_a_stranger_is_a_403() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;

    for body in [
        json!({ "kind": "portal_theme" }),
        json!({ "kind": "portal_theme", "revision": 1, "realm": "master" }),
        json!({ "kind": "portal_theme", "revision": 1, "realm": "master", "items": [{ "phase": "Exploded" }] }),
        json!({ "kind": "portal_theme", "revision": 1, "realm": "master",
            "items": [{ "phase": "Applied" }, { "phase": "Applied" }] }),
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
            "kind": "portal_theme", "revision": 1, "realm": "master",
            "items": [{ "phase": "Applied" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    env.forget().await;
}

#[tokio::test]
async fn the_data_plane_gets_the_tokens_and_the_stored_action_keeps_them_once_published() {
    let Some(pool) = pool().await else {
        return;
    };
    let env = Env::new(pool, "ferriskey", BRANDING).await;
    let id = env.made("Autumn", "#112233").await;
    env.activate(&id).await;

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
    assert_eq!(claimed[0]["action_type"], "deployment.portal_theme");
    assert_eq!(claimed[0]["payload"]["data"]["config"], tokens("#112233"));

    env.publish_everything_claimed(claimed).await;

    let stored = env.sent().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["config"], tokens("#112233"));

    let (status, listed) = call(&env.owner_app(), Method::GET, &env.actions(), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"][0]["payload"]["data"]["realm"], "master");

    env.forget().await;
}

impl Env {
    async fn publish_everything_claimed(&self, claimed: &[Value]) {
        let published: Vec<&str> = claimed
            .iter()
            .map(|action| action["id"].as_str().unwrap())
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
}
