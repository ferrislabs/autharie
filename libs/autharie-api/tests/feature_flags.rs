use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::{
        organisations::enabled_features::list_enabled_features_handler,
        platform::features::{
            list_platform_features_handler, list_tenant_features_handler,
            set_feature_default_handler, set_tenant_feature_handler,
        },
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

struct Person {
    id: Uuid,
}

impl Person {
    async fn seed(pool: &PgPool, rights: &[&str]) -> Self {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'flags', $3)")
            .bind(id)
            .bind(format!("{id}@flags-api.test"))
            .bind(id.to_string())
            .execute(pool)
            .await
            .expect("a user");

        if !rights.is_empty() {
            sqlx::query(
                "INSERT INTO platform_operators (subject, rights, granted_by, granted_at) \
                 VALUES ($1, $2, NULL, now())",
            )
            .bind(id.to_string())
            .bind(rights.iter().map(ToString::to_string).collect::<Vec<_>>())
            .execute(pool)
            .await
            .expect("an operator");
        }

        Self { id }
    }

    fn identity(&self) -> Identity {
        Identity::User(User {
            id: self.id.to_string(),
            username: "flags".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })
    }

    async fn forget(&self, pool: &PgPool) {
        sqlx::query("DELETE FROM fleet_audit_log WHERE actor_subject = $1")
            .bind(self.id.to_string())
            .execute(pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM platform_operators WHERE subject = $1")
            .bind(self.id.to_string())
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

async fn organisation(pool: &PgPool, owner: &Person) -> Uuid {
    let organisation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'flags', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
    )
    .bind(organisation)
    .bind(format!("flags-api-{organisation}"))
    .bind(owner.id)
    .execute(pool)
    .await
    .expect("an organisation");

    organisation
}

async fn forget(pool: &PgPool, organisation: Uuid, people: &[&Person]) {
    sqlx::query("DELETE FROM organisations WHERE id = $1")
        .bind(organisation)
        .execute(pool)
        .await
        .expect("cleanup");
    for person in people {
        person.forget(pool).await;
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(person.id)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

fn app(pool: &PgPool, person: &Person) -> Router {
    Router::new()
        .typed_get(list_platform_features_handler)
        .typed_put(set_feature_default_handler)
        .typed_get(list_tenant_features_handler)
        .typed_put(set_tenant_feature_handler)
        .typed_get(list_enabled_features_handler)
        .layer(Extension(person.identity()))
        .with_state(AppState {
            args: Arc::new(Args::default()),
            service: AutharieService::new(pool.clone()),
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
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    (status, value)
}

fn tenant_uri(organisation: Uuid) -> String {
    format!("/platform/organisations/{organisation}/features")
}

#[tokio::test]
async fn every_endpoint_refuses_an_operator_without_its_right() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let only_acting = Person::seed(&pool, &["act_on_tenant"]).await;
    let only_viewing = Person::seed(&pool, &["view_estate"]).await;
    let nobody = Person::seed(&pool, &[]).await;
    let on_the_default = json!({ "enabled_by_default": true });
    let on_the_tenant = json!({ "enabled": true });

    let cases = [
        (
            Method::GET,
            "/platform/features".to_string(),
            None,
            &only_acting,
        ),
        (
            Method::PUT,
            "/platform/features/customer_cloud".to_string(),
            Some(on_the_default.clone()),
            &only_viewing,
        ),
        (Method::GET, tenant_uri(organisation), None, &only_acting),
        (
            Method::PUT,
            format!("{}/customer_cloud", tenant_uri(organisation)),
            Some(on_the_tenant.clone()),
            &only_viewing,
        ),
        (Method::GET, "/platform/features".to_string(), None, &nobody),
        (
            Method::PUT,
            "/platform/features/customer_cloud".to_string(),
            Some(on_the_default),
            &nobody,
        ),
        (
            Method::PUT,
            format!("{}/customer_cloud", tenant_uri(organisation)),
            Some(on_the_tenant),
            &nobody,
        ),
    ];

    let mut refused = Vec::new();
    for (method, uri, body, person) in cases {
        let (status, _) = call(&app(&pool, person), method.clone(), &uri, body).await;
        refused.push((method, uri, status));
    }

    let (_, after) = call(
        &app(&pool, &only_viewing),
        Method::GET,
        &tenant_uri(organisation),
        None,
    )
    .await;
    forget(
        &pool,
        organisation,
        &[&owner, &only_acting, &only_viewing, &nobody],
    )
    .await;

    for (method, uri, status) in refused {
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    assert_eq!(
        after["data"][0]["override"],
        Value::Null,
        "a refused write left an override behind"
    );
}

#[tokio::test]
async fn an_unknown_key_is_not_found() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "operate_fleet", "act_on_tenant"]).await;
    let app = app(&pool, &operator);

    let (default, _) = call(
        &app,
        Method::PUT,
        "/platform/features/not_a_feature",
        Some(json!({ "enabled_by_default": true })),
    )
    .await;
    let (tenant, _) = call(
        &app,
        Method::PUT,
        &format!("{}/not_a_feature", tenant_uri(organisation)),
        Some(json!({ "enabled": true })),
    )
    .await;
    forget(&pool, organisation, &[&owner, &operator]).await;

    assert_eq!(default, StatusCode::NOT_FOUND);
    assert_eq!(tenant, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_organisation_is_not_found() {
    let Some(pool) = pool().await else {
        return;
    };
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let app = app(&pool, &operator);
    let nobody = Uuid::new_v4();

    let (listed, _) = call(&app, Method::GET, &tenant_uri(nobody), None).await;
    let (set, _) = call(
        &app,
        Method::PUT,
        &format!("{}/customer_cloud", tenant_uri(nobody)),
        Some(json!({ "enabled": true })),
    )
    .await;
    forget(&pool, Uuid::new_v4(), &[&operator]).await;

    assert_eq!(listed, StatusCode::NOT_FOUND);
    assert_eq!(set, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_override_is_set_read_and_cleared_through_the_api() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let app = app(&pool, &operator);
    let uri = format!("{}/customer_cloud", tenant_uri(organisation));

    let (status, switched_off) =
        call(&app, Method::PUT, &uri, Some(json!({ "enabled": false }))).await;
    assert_eq!(status, StatusCode::OK, "{switched_off}");
    let (_, listed_off) = call(&app, Method::GET, &tenant_uri(organisation), None).await;

    let (_, switched_on) = call(&app, Method::PUT, &uri, Some(json!({ "enabled": true }))).await;
    let (_, listed_on) = call(&app, Method::GET, &tenant_uri(organisation), None).await;

    let (status, cleared) = call(&app, Method::PUT, &uri, Some(json!({ "enabled": null }))).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    let (_, listed_cleared) = call(&app, Method::GET, &tenant_uri(organisation), None).await;

    let (missing, _) = call(&app, Method::PUT, &uri, Some(json!({}))).await;
    forget(&pool, organisation, &[&owner, &operator]).await;

    assert_eq!(switched_off["data"]["key"], "customer_cloud");
    assert_eq!(switched_off["data"]["override"], false);
    assert_eq!(switched_off["data"]["effective"], false);
    assert!(switched_off["data"]["description"].as_str().is_some());
    assert_eq!(listed_off["data"][0]["override"], false);

    assert_eq!(switched_on["data"]["override"], true);
    assert_eq!(switched_on["data"]["effective"], true);
    assert_eq!(listed_on["data"][0]["effective"], true);

    assert_eq!(cleared["data"]["override"], Value::Null);
    assert_eq!(
        cleared["data"]["effective"], cleared["data"]["default"],
        "without an override the default applies"
    );
    assert_eq!(listed_cleared["data"][0]["override"], Value::Null);

    assert_eq!(
        missing,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a body that does not say enabled must not clear the override"
    );
}

#[tokio::test]
async fn the_default_is_set_and_the_overrides_are_counted() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "operate_fleet", "act_on_tenant"]).await;
    let app = app(&pool, &operator);

    let (_, before) = call(&app, Method::GET, "/platform/features", None).await;
    let was = before["data"][0]["enabled_by_default"]
        .as_bool()
        .expect("a default");
    let on_before = before["data"][0]["overrides_on"].as_u64().expect("a count");

    call(
        &app,
        Method::PUT,
        &format!("{}/customer_cloud", tenant_uri(organisation)),
        Some(json!({ "enabled": true })),
    )
    .await;
    let (status, changed) = call(
        &app,
        Method::PUT,
        "/platform/features/customer_cloud",
        Some(json!({ "enabled_by_default": !was })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    let (_, listed) = call(&app, Method::GET, "/platform/features", None).await;

    call(
        &app,
        Method::PUT,
        "/platform/features/customer_cloud",
        Some(json!({ "enabled_by_default": was })),
    )
    .await;
    let trail: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fleet_audit_log \
         WHERE actor_subject = $1 AND target_feature = 'customer_cloud' \
         AND action = 'feature.default_changed'",
    )
    .bind(operator.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("the trail");
    forget(&pool, organisation, &[&owner, &operator]).await;

    assert_eq!(changed["data"]["enabled_by_default"], !was);
    assert_eq!(listed["data"][0]["key"], "customer_cloud");
    assert_eq!(listed["data"][0]["enabled_by_default"], !was);
    assert!(
        listed["data"][0]["overrides_on"].as_u64().expect("a count") > on_before,
        "the override just set is counted"
    );
    assert_eq!(trail, 2, "each change of the default is in the fleet trail");
}

#[tokio::test]
async fn a_member_reads_the_effective_keys_and_a_stranger_is_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let stranger = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let uri = format!("/organisations/{organisation}/feature-flags");
    let flag = format!("{}/customer_cloud", tenant_uri(organisation));

    call(
        &app(&pool, &operator),
        Method::PUT,
        &flag,
        Some(json!({ "enabled": false })),
    )
    .await;
    let (off_status, off) = call(&app(&pool, &owner), Method::GET, &uri, None).await;

    call(
        &app(&pool, &operator),
        Method::PUT,
        &flag,
        Some(json!({ "enabled": true })),
    )
    .await;
    let (on_status, on) = call(&app(&pool, &owner), Method::GET, &uri, None).await;
    let (stranger_status, _) = call(&app(&pool, &stranger), Method::GET, &uri, None).await;
    forget(&pool, organisation, &[&owner, &stranger, &operator]).await;

    assert_eq!(off_status, StatusCode::OK);
    assert_eq!(off, json!({ "enabled": [] }));
    assert_eq!(on_status, StatusCode::OK);
    assert_eq!(on, json!({ "enabled": ["customer_cloud"] }));
    assert_eq!(stranger_status, StatusCode::FORBIDDEN);
}
