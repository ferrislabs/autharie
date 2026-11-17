use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::platform::{
        features::{list_tenant_features_handler, set_tenant_feature_handler},
        offers::{list_tenant_offers_handler, set_tenant_offer_handler},
        organisation_quotas::{
            list_tenant_organisation_quotas_handler, set_tenant_organisation_quota_handler,
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
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'limits', $3)")
            .bind(id)
            .bind(format!("{id}@limits-api.test"))
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
            scopes: Vec::new(),
            act: None,
            id: self.id.to_string(),
            username: "limits".to_string(),
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
         VALUES ($1, 'limits', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
    )
    .bind(organisation)
    .bind(format!("limits-api-{organisation}"))
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
        .typed_get(list_tenant_features_handler)
        .typed_put(set_tenant_feature_handler)
        .typed_get(list_tenant_organisation_quotas_handler)
        .typed_put(set_tenant_organisation_quota_handler)
        .typed_get(list_tenant_offers_handler)
        .typed_put(set_tenant_offer_handler)
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

type TrailRow = (String, Option<Value>, Option<Value>, Option<Uuid>);

fn quotas_uri(organisation: Uuid) -> String {
    format!("/platform/organisations/{organisation}/organisation-quotas")
}

fn offers_uri(organisation: Uuid) -> String {
    format!("/platform/organisations/{organisation}/offers")
}

fn features_uri(organisation: Uuid) -> String {
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
    let quota = json!({ "quota": { "type": "limited", "max": 9 } });
    let offer = json!({ "open": true });

    let quota_uri = format!("{}/deployments", quotas_uri(organisation));
    let offer_uri = format!("{}/sandbox", offers_uri(organisation));
    let cases = [
        (Method::GET, quotas_uri(organisation), None, &only_acting),
        (
            Method::PUT,
            quota_uri.clone(),
            Some(quota.clone()),
            &only_viewing,
        ),
        (Method::PUT, quota_uri, Some(quota), &nobody),
        (Method::GET, offers_uri(organisation), None, &only_acting),
        (
            Method::PUT,
            offer_uri.clone(),
            Some(offer.clone()),
            &only_viewing,
        ),
        (Method::PUT, offer_uri, Some(offer), &nobody),
    ];

    let mut refused = Vec::new();
    for (method, uri, body, person) in cases {
        let (status, _) = call(&app(&pool, person), method.clone(), &uri, body).await;
        refused.push((method, uri, status));
    }
    let (_, quotas) = call(
        &app(&pool, &only_viewing),
        Method::GET,
        &quotas_uri(organisation),
        None,
    )
    .await;
    let (_, offers) = call(
        &app(&pool, &only_viewing),
        Method::GET,
        &offers_uri(organisation),
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
    assert!(
        quotas["data"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|entry| entry["override"].is_null()),
        "a refused write left an override behind"
    );
    assert!(
        offers["data"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|entry| entry["override"].is_null()),
        "a refused write left an override behind"
    );
}

#[tokio::test]
async fn an_unknown_organisation_kind_or_offer_is_not_found() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let app = app(&pool, &operator);
    let nobody = Uuid::new_v4();
    let quota = json!({ "quota": { "type": "unlimited" } });

    let mut statuses = Vec::new();
    for (method, uri, body) in [
        (Method::GET, quotas_uri(nobody), None),
        (
            Method::PUT,
            format!("{}/deployments", quotas_uri(nobody)),
            Some(quota.clone()),
        ),
        (
            Method::PUT,
            format!("{}/mcp_servers", quotas_uri(organisation)),
            Some(quota.clone()),
        ),
        (
            Method::PUT,
            format!("{}/nonsense", quotas_uri(organisation)),
            Some(quota),
        ),
        (Method::GET, offers_uri(nobody), None),
        (
            Method::PUT,
            format!("{}/sandbox", offers_uri(nobody)),
            Some(json!({ "open": true })),
        ),
        (
            Method::PUT,
            format!("{}/nonsense", offers_uri(organisation)),
            Some(json!({ "open": true })),
        ),
    ] {
        let (status, _) = call(&app, method, &uri, body).await;
        statuses.push(status);
    }
    forget(&pool, organisation, &[&owner, &operator]).await;

    assert_eq!(statuses, vec![StatusCode::NOT_FOUND; 7]);
}

#[tokio::test]
async fn an_organisation_quota_is_listed_set_and_cleared() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    sqlx::query(
        "INSERT INTO members (id, organisation_id, user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(organisation)
    .bind(owner.id)
    .execute(&pool)
    .await
    .expect("a membership");
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let app = app(&pool, &operator);
    let uri = format!("{}/deployments", quotas_uri(organisation));

    let (status, listed) = call(&app, Method::GET, &quotas_uri(organisation), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");

    let (status, set) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({ "quota": { "type": "limited", "max": 2 } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{set}");
    let (_, unlimited) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({ "quota": { "type": "unlimited" } })),
    )
    .await;
    let (status, cleared) = call(&app, Method::PUT, &uri, Some(json!({ "quota": null }))).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    let (missing, _) = call(&app, Method::PUT, &uri, Some(json!({}))).await;
    forget(&pool, organisation, &[&owner, &operator]).await;

    assert_eq!(listed["data"][0]["kind"], "deployments");
    assert_eq!(
        listed["data"][0]["plan_default"],
        json!({ "type": "limited", "max": 1 })
    );
    assert_eq!(listed["data"][0]["override"], Value::Null);
    assert_eq!(
        listed["data"][0]["effective"],
        json!({ "type": "limited", "max": 1 })
    );
    assert_eq!(listed["data"][0]["held"], 0);
    assert!(listed["data"][0]["description"].as_str().is_some());
    assert_eq!(listed["data"][1]["kind"], "members");
    assert_eq!(
        listed["data"][1]["plan_default"],
        json!({ "type": "limited", "max": 2 })
    );
    assert_eq!(listed["data"][1]["held"], 1);

    assert_eq!(
        set["data"]["override"],
        json!({ "type": "limited", "max": 2 })
    );
    assert_eq!(
        set["data"]["effective"],
        json!({ "type": "limited", "max": 2 })
    );
    assert_eq!(
        unlimited["data"]["effective"],
        json!({ "type": "unlimited" })
    );
    assert_eq!(cleared["data"]["override"], Value::Null);
    assert_eq!(
        cleared["data"]["effective"],
        json!({ "type": "limited", "max": 1 })
    );
    assert_eq!(missing, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn an_offer_is_listed_opened_closed_and_cleared() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let app = app(&pool, &operator);
    let uri = format!("{}/sandbox", offers_uri(organisation));

    let (status, listed) = call(&app, Method::GET, &offers_uri(organisation), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let (status, opened) = call(&app, Method::PUT, &uri, Some(json!({ "open": true }))).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let (_, listed_open) = call(&app, Method::GET, &offers_uri(organisation), None).await;
    let (_, closed) = call(
        &app,
        Method::PUT,
        &format!("{}/free", offers_uri(organisation)),
        Some(json!({ "open": false })),
    )
    .await;
    let (_, cleared) = call(&app, Method::PUT, &uri, Some(json!({ "open": null }))).await;
    let (missing, _) = call(&app, Method::PUT, &uri, Some(json!({}))).await;
    forget(&pool, organisation, &[&owner, &operator]).await;

    let offers = listed["data"].as_array().expect("a list");
    assert_eq!(offers.len(), 5);
    let sandbox = offers
        .iter()
        .find(|entry| entry["offer"] == "sandbox")
        .expect("sandbox");
    assert_eq!(
        (
            &sandbox["plan_default"],
            &sandbox["override"],
            &sandbox["effective"]
        ),
        (&json!(false), &Value::Null, &json!(false))
    );
    let free = offers
        .iter()
        .find(|entry| entry["offer"] == "free")
        .expect("free");
    assert_eq!(free["plan_default"], true);
    assert_eq!(free["effective"], true);

    assert_eq!(
        opened["data"],
        json!({ "offer": "sandbox", "plan_default": false, "override": true, "effective": true })
    );
    let sandbox_open = listed_open["data"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|entry| entry["offer"] == "sandbox")
        .expect("sandbox");
    assert_eq!(sandbox_open["effective"], true);
    assert_eq!(
        closed["data"],
        json!({ "offer": "free", "plan_default": true, "override": false, "effective": false })
    );
    assert_eq!(
        cleared["data"],
        json!({ "offer": "sandbox", "plan_default": false, "override": null, "effective": false })
    );
    assert_eq!(missing, StatusCode::UNPROCESSABLE_ENTITY);
}

/// @spec-pilot-6
#[tokio::test]
async fn a_quota_an_offer_and_a_feature_change_each_reach_the_audit_trail() {
    let Some(pool) = pool().await else {
        return;
    };
    let owner = Person::seed(&pool, &[]).await;
    let organisation = organisation(&pool, &owner).await;
    let operator = Person::seed(&pool, &["view_estate", "act_on_tenant"]).await;
    let app = app(&pool, &operator);

    call(
        &app,
        Method::PUT,
        &format!("{}/members", quotas_uri(organisation)),
        Some(json!({ "quota": { "type": "limited", "max": 4 } })),
    )
    .await;
    call(
        &app,
        Method::PUT,
        &format!("{}/sandbox", offers_uri(organisation)),
        Some(json!({ "open": true })),
    )
    .await;
    call(
        &app,
        Method::PUT,
        &format!("{}/agents", features_uri(organisation)),
        Some(json!({ "enabled": true })),
    )
    .await;

    let trail: Vec<TrailRow> = sqlx::query_as(
        "SELECT action, change_before, change_after, actor_user_id FROM audit_log \
         WHERE organisation_id = $1 ORDER BY recorded_at",
    )
    .bind(organisation)
    .fetch_all(&pool)
    .await
    .expect("the trail");
    forget(&pool, organisation, &[&owner, &operator]).await;

    let actions: Vec<_> = trail.iter().map(|(action, ..)| action.as_str()).collect();
    assert_eq!(
        actions,
        [
            "organisation.quota.changed",
            "organisation.offer.changed",
            "organisation.feature.changed"
        ]
    );
    assert!(trail.iter().all(|entry| entry.3 == Some(operator.id)));
    assert_eq!(
        trail[0].2,
        Some(json!({ "quota": "members", "override": { "type": "limited", "max": 4 } }))
    );
    assert_eq!(
        trail[1].1,
        Some(json!({ "offer": "sandbox", "override": null }))
    );
    assert_eq!(
        trail[1].2,
        Some(json!({ "offer": "sandbox", "override": true }))
    );
}
