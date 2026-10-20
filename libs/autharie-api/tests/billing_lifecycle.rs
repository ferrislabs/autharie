use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::deployments::{
        create_deployment::create_deployment_handler, delete_deployment::delete_deployment_handler,
        iam_settings::set_iam_settings_handler,
    },
    state::AppState,
};
use autharie_auth::{Identity, User};
use autharie_core::{
    AutharieService,
    dataplane::{
        ports::DataPlaneService,
        value_objects::{Capacity, CreateDataplaneCommand, DataPlaneAllocation, Region},
    },
};
use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use axum_extra::routing::RouterExt;
use chrono::{DateTime, Utc};
use futures::FutureExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const GATEWAY: &str = "203.0.113.7";

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

async fn a_user(pool: &PgPool, id: Uuid) {
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'billing', $3)")
        .bind(id)
        .bind(format!("{id}@billing-lifecycle.test"))
        .bind(id.to_string())
        .execute(pool)
        .await
        .expect("a user");
}

async fn an_organisation(pool: &PgPool, id: Uuid, owner: Uuid, plan: &str, max_instances: i32) {
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'billing', $2, $3, 'active', $4, $5, 2, 1, now(), now())",
    )
    .bind(id)
    .bind(format!("bl-{id}"))
    .bind(owner)
    .bind(plan)
    .bind(max_instances)
    .execute(pool)
    .await
    .expect("an organisation");
}

type SubscriptionRow = (
    Uuid,
    String,
    i64,
    String,
    Vec<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

struct Installation {
    pool: PgPool,
    operator: Uuid,
    owner: Uuid,
    free_organisation: Uuid,
    paid_organisation: Uuid,
    region: String,
    release: String,
}

impl Installation {
    async fn seed(pool: PgPool) -> Self {
        let installation = Self {
            pool,
            operator: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            free_organisation: Uuid::new_v4(),
            paid_organisation: Uuid::new_v4(),
            region: format!("billing-lc-{}", Uuid::new_v4()),
            release: format!("0.9.{}", Uuid::new_v4().as_u128() as u32 % 1_000_000),
        };
        let pool = &installation.pool;

        a_user(pool, installation.operator).await;
        a_user(pool, installation.owner).await;
        an_organisation(
            pool,
            installation.free_organisation,
            installation.owner,
            "free",
            1,
        )
        .await;
        an_organisation(
            pool,
            installation.paid_organisation,
            installation.owner,
            "starter",
            5,
        )
        .await;
        sqlx::query(
            "INSERT INTO releases (kind, version, status, risk) \
             VALUES ('ferriskey', $1, 'available', 'none')",
        )
        .bind(&installation.release)
        .execute(pool)
        .await
        .expect("a release");

        installation
    }

    fn service(&self) -> AutharieService {
        AutharieService::new(self.pool.clone())
    }

    async fn serve_region(&self) {
        let service = self.service();
        service
            .ensure_first_operator(&self.operator.to_string())
            .await
            .expect("the operator holds every right");
        let registered = service
            .create_dataplane(
                user(self.operator),
                CreateDataplaneCommand {
                    region: Region::new(self.region.clone()),
                    allocation: DataPlaneAllocation::Shared,
                    capacity: Capacity::new(64_000, 131_072, 2_000).expect("a capacity"),
                },
            )
            .await
            .expect("a shared data plane");
        sqlx::query(
            "UPDATE data_planes SET status = 'active', last_seen_at = now(), \
             herald_client_id = $2, herald_subject = $3, gateway_address = $4 WHERE id = $1",
        )
        .bind(registered.dataplane.id.0)
        .bind(format!("herald-{}", registered.dataplane.id.0))
        .bind(Uuid::new_v4().to_string())
        .bind(GATEWAY)
        .execute(&self.pool)
        .await
        .expect("a data plane that reports");
    }

    fn app(&self) -> Router {
        let mut args = Args::default();
        args.dataplane.default_region = self.region.clone();

        Router::new()
            .typed_post(create_deployment_handler)
            .typed_delete(delete_deployment_handler)
            .typed_put(set_iam_settings_handler)
            .layer(Extension(user(self.owner)))
            .with_state(AppState {
                args: Arc::new(args),
                service: self.service(),
                certificate_source: None,
                quickwit_search: None,
                quickwit_traces: None,
            })
    }

    fn deployments(&self, organisation: Uuid) -> String {
        format!("/organisations/{organisation}/deployments")
    }

    fn deployment(&self, organisation: Uuid, deployment: Uuid) -> String {
        format!("{}/{deployment}", self.deployments(organisation))
    }

    fn command(&self, label: &str, offer: &str) -> Value {
        json!({
            "name": format!("{label}-{}", &Uuid::new_v4().simple().to_string()[..12]),
            "kind": "ferriskey",
            "version": self.release,
            "environment": "production",
            "offer": offer,
        })
    }

    async fn create(&self, organisation: Uuid, label: &str, offer: &str) -> Uuid {
        let (status, created) = call(
            &self.app(),
            Method::POST,
            &self.deployments(organisation),
            Some(self.command(label, offer)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");

        created["data"]["id"]
            .as_str()
            .expect("an id")
            .parse()
            .expect("a uuid")
    }

    async fn created_at(&self, deployment: Uuid) -> DateTime<Utc> {
        sqlx::query_scalar("SELECT created_at FROM deployments WHERE id = $1")
            .bind(deployment)
            .fetch_one(&self.pool)
            .await
            .expect("the deployment")
    }

    async fn subscriptions_of(&self, deployment: Uuid) -> Vec<SubscriptionRow> {
        sqlx::query_as(
            "SELECT organisation_id, tier_code, monthly_price_minor, offer, features, \
             starts_at, ends_at FROM billing_subscriptions \
             WHERE deployment_id = $1 ORDER BY starts_at",
        )
        .bind(deployment)
        .fetch_all(&self.pool)
        .await
        .expect("the subscriptions")
    }

    async fn features_of_tier(&self, code: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT features FROM billing_tiers WHERE code = $1")
            .bind(code)
            .fetch_one(&self.pool)
            .await
            .expect("the tier")
    }

    async fn iam_requests_of(&self, deployment: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM actions WHERE deployment_id = $1 AND action_type <> 'deployment.create'",
        )
        .bind(deployment)
        .fetch_one(&self.pool)
        .await
        .expect("the actions")
    }

    async fn forget(&self) {
        for organisation in [self.free_organisation, self.paid_organisation] {
            for statement in [
                "DELETE FROM billing_subscriptions WHERE organisation_id = $1",
                "DELETE FROM actions WHERE deployment_id IN \
                 (SELECT id FROM deployments WHERE organisation_id = $1)",
                "DELETE FROM backup_schedules WHERE deployment_id IN \
                 (SELECT id FROM deployments WHERE organisation_id = $1)",
                "DELETE FROM deployments WHERE organisation_id = $1",
                "DELETE FROM organisations WHERE id = $1",
            ] {
                sqlx::query(statement)
                    .bind(organisation)
                    .execute(&self.pool)
                    .await
                    .expect("cleanup");
            }
        }
        for statement in [
            "DELETE FROM actions WHERE dataplane_id IN \
             (SELECT id FROM data_planes WHERE region LIKE $1 || '%')",
            "DELETE FROM data_planes WHERE region LIKE $1 || '%'",
        ] {
            sqlx::query(statement)
                .bind(&self.region)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("DELETE FROM releases WHERE kind = 'ferriskey' AND version = $1")
            .bind(&self.release)
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM platform_operators WHERE subject = $1")
            .bind(self.operator.to_string())
            .execute(&self.pool)
            .await
            .expect("cleanup");
        for id in [self.operator, self.owner] {
            sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
    }
}

fn user(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "billing".to_string(),
        email: None,
        name: None,
        roles: vec![],
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
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    (status, value)
}

async fn scenario<F>(body: F)
where
    F: AsyncFnOnce(&Installation),
{
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool).await;

    let outcome = std::panic::AssertUnwindSafe(body(&installation))
        .catch_unwind()
        .await;
    installation.forget().await;

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

fn sorted(mut features: Vec<String>) -> Vec<String> {
    features.sort();
    features
}

fn branding() -> Value {
    json!({ "branding": { "colors": { "primary": "#112233" }, "radius": 6 } })
}

#[tokio::test]
async fn creating_a_deployment_opens_a_subscription_on_the_legacy_tier_of_its_offer() {
    scenario(async |installation| {
        installation.serve_region().await;

        for (organisation, offer) in [
            (installation.free_organisation, "free"),
            (installation.paid_organisation, "standard"),
        ] {
            let deployment = installation.create(organisation, "opened", offer).await;

            let subscriptions = installation.subscriptions_of(deployment).await;
            assert_eq!(subscriptions.len(), 1, "{offer}: {subscriptions:?}");
            let (owner, tier, price, taken, features, starts_at, ends_at) = &subscriptions[0];

            assert_eq!(*owner, organisation, "{offer}");
            assert_eq!(tier, &format!("legacy-{offer}"));
            assert_eq!(*price, 0, "{offer}");
            assert_eq!(taken, offer);
            assert_eq!(*ends_at, None, "{offer}: it is open");
            assert_eq!(
                *starts_at,
                installation.created_at(deployment).await,
                "{offer}: it starts when the deployment does"
            );
            assert_eq!(
                sorted(features.clone()),
                sorted(installation.features_of_tier(tier).await),
                "{offer}: the tier's features are copied"
            );
            assert!(features.iter().any(|feature| feature == "branding"));
        }
    })
    .await;
}

#[tokio::test]
async fn a_refused_creation_opens_no_subscription() {
    scenario(async |installation| {
        installation.serve_region().await;
        let before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM billing_subscriptions WHERE organisation_id = $1",
        )
        .bind(installation.free_organisation)
        .fetch_one(&installation.pool)
        .await
        .expect("count");

        let (status, refused) = call(
            &installation.app(),
            Method::POST,
            &installation.deployments(installation.free_organisation),
            Some(installation.command("refused", "sandbox")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");

        let after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM billing_subscriptions WHERE organisation_id = $1",
        )
        .bind(installation.free_organisation)
        .fetch_one(&installation.pool)
        .await
        .expect("count");
        assert_eq!(before, after);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_deployment_closes_its_subscription_at_the_deletion() {
    scenario(async |installation| {
        installation.serve_region().await;
        let organisation = installation.paid_organisation;
        let deployment = installation
            .create(organisation, "closed", "standard")
            .await;
        let opened = installation.subscriptions_of(deployment).await;
        assert_eq!(opened.len(), 1, "{opened:?}");

        let started = Utc::now();
        let (status, deleted) = call(
            &installation.app(),
            Method::DELETE,
            &installation.deployment(organisation, deployment),
            None,
        )
        .await;
        let finished = Utc::now();
        assert_eq!(status, StatusCode::OK, "{deleted}");

        let subscriptions = installation.subscriptions_of(deployment).await;
        assert_eq!(subscriptions.len(), 1, "{subscriptions:?}");
        let (_, tier, _, _, _, starts_at, ends_at) = &subscriptions[0];
        let ends_at = ends_at.expect("the subscription is closed");

        assert_eq!(tier, "legacy-standard");
        assert_eq!(*starts_at, opened[0].5, "closing does not move the start");
        assert!(
            started <= ends_at && ends_at <= finished,
            "{started} <= {ends_at} <= {finished}"
        );

        let still_open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM billing_subscriptions \
             WHERE deployment_id = $1 AND ends_at IS NULL",
        )
        .bind(deployment)
        .fetch_one(&installation.pool)
        .await
        .expect("count");
        assert_eq!(still_open, 0);
    })
    .await;
}

#[tokio::test]
async fn a_deployment_without_a_subscription_is_still_deleted() {
    scenario(async |installation| {
        installation.serve_region().await;
        let organisation = installation.paid_organisation;
        let deployment = installation.create(organisation, "bare", "standard").await;
        sqlx::query("DELETE FROM billing_subscriptions WHERE deployment_id = $1")
            .bind(deployment)
            .execute(&installation.pool)
            .await
            .expect("a deployment older than billing");

        let (status, deleted) = call(
            &installation.app(),
            Method::DELETE,
            &installation.deployment(organisation, deployment),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{deleted}");
        assert!(installation.subscriptions_of(deployment).await.is_empty());
        let removed: bool =
            sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM deployments WHERE id = $1")
                .bind(deployment)
                .fetch_one(&installation.pool)
                .await
                .expect("the deployment");
        assert!(removed);
    })
    .await;
}

#[tokio::test]
async fn a_legacy_tier_opens_branding_on_the_deployment() {
    scenario(async |installation| {
        installation.serve_region().await;
        let organisation = installation.paid_organisation;
        let deployment = installation
            .create(organisation, "branded", "standard")
            .await;

        let (status, accepted) = call(
            &installation.app(),
            Method::PUT,
            &format!(
                "{}/iam-settings",
                installation.deployment(organisation, deployment)
            ),
            Some(branding()),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
        assert!(accepted["data"]["branding"].is_object(), "{accepted}");
        assert_eq!(installation.iam_requests_of(deployment).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_subscription_that_does_not_open_branding_refuses_it() {
    scenario(async |installation| {
        installation.serve_region().await;
        let organisation = installation.paid_organisation;
        let deployment = installation.create(organisation, "plain", "standard").await;
        sqlx::query(
            "UPDATE billing_subscriptions SET features = ARRAY['mfa'] WHERE deployment_id = $1",
        )
        .bind(deployment)
        .execute(&installation.pool)
        .await
        .expect("a subscription without branding");

        let (status, refused) = call(
            &installation.app(),
            Method::PUT,
            &format!(
                "{}/iam-settings",
                installation.deployment(organisation, deployment)
            ),
            Some(branding()),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert!(refused.to_string().contains("legacy-standard"), "{refused}");
        assert!(refused.to_string().contains("branding"), "{refused}");
        assert_eq!(installation.iam_requests_of(deployment).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_deployment_without_a_subscription_cannot_be_branded() {
    scenario(async |installation| {
        installation.serve_region().await;
        let organisation = installation.paid_organisation;
        let deployment = installation
            .create(organisation, "orphan", "standard")
            .await;
        sqlx::query("DELETE FROM billing_subscriptions WHERE deployment_id = $1")
            .bind(deployment)
            .execute(&installation.pool)
            .await
            .expect("a deployment older than billing");

        let (status, refused) = call(
            &installation.app(),
            Method::PUT,
            &format!(
                "{}/iam-settings",
                installation.deployment(organisation, deployment)
            ),
            Some(branding()),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert!(refused.to_string().contains("no subscription"), "{refused}");
        assert_eq!(installation.iam_requests_of(deployment).await, 0);
    })
    .await;
}
