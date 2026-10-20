use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args, handlers::deployments::create_deployment::create_deployment_handler,
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
use futures::FutureExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const ZONE: &str = "tenants.autharie.test";
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
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'free', $3)")
        .bind(id)
        .bind(format!("{id}@free-api.test"))
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
         VALUES ($1, 'free', $2, $3, 'active', $4, $5, 2, 1, now(), now())",
    )
    .bind(id)
    .bind(format!("free-api-{id}"))
    .bind(owner)
    .bind(plan)
    .bind(max_instances)
    .execute(pool)
    .await
    .expect("an organisation");
}

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
            region: format!("free-api-{}", Uuid::new_v4()),
            release: format!("0.8.{}", Uuid::new_v4().as_u128() as u32 % 1_000_000),
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
            .with_domain(Some(ZONE.to_string()))
            .with_platform_labels(vec!["control".to_string()])
    }

    async fn serve_region(&self) -> Uuid {
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

        registered.dataplane.id.0
    }

    fn app(&self) -> Router {
        let mut args = Args::default();
        args.dataplane.default_region = self.region.clone();

        Router::new()
            .typed_post(create_deployment_handler)
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

    fn command(&self, name: &str, kind: &str, offer: &str) -> Value {
        json!({
            "name": name,
            "kind": kind,
            "version": self.release,
            "environment": "production",
            "offer": offer,
        })
    }

    async fn actions_of(&self, deployment: &str) -> Vec<(String, Value)> {
        let deployment: Uuid = deployment.parse().expect("a uuid");
        sqlx::query_as(
            "SELECT action_type, payload FROM actions WHERE deployment_id = $1 ORDER BY created_at",
        )
        .bind(deployment)
        .fetch_all(&self.pool)
        .await
        .expect("the actions")
    }

    async fn forget(&self) {
        for organisation in [self.free_organisation, self.paid_organisation] {
            for statement in [
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
        username: "free".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

async fn call(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
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

/// @spec-free-1
#[tokio::test]
async fn a_free_organisation_gets_an_instance_on_a_shared_data_plane() {
    scenario(async |installation| {
        let plane = installation.serve_region().await;

        let (status, created) = call(
            &installation.app(),
            &installation.deployments(installation.free_organisation),
            installation.command("Acme Login", "ferriskey", "free"),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let deployment = &created["data"];
        assert_eq!(deployment["offer"], "free");
        assert_eq!(deployment["distribution"], "shared", "{deployment}");

        let id = deployment["id"].as_str().expect("an id");
        let placed_on: Uuid =
            sqlx::query_scalar("SELECT dataplane_id FROM deployments WHERE id = $1")
                .bind(id.parse::<Uuid>().expect("a uuid"))
                .fetch_one(&installation.pool)
                .await
                .expect("the stored plane");
        assert_eq!(placed_on, plane);

        let actions = installation.actions_of(id).await;
        assert_eq!(actions.len(), 1, "{actions:?}");
        let (action_type, payload) = &actions[0];
        assert_eq!(action_type, "deployment.create");
        assert_eq!(payload["database"], "shared");
        assert_eq!(payload["cpu_millis"], 100);
        assert_eq!(payload["memory_mib"], 256);
        assert_eq!(payload["storage_gib"], 1);
        assert_eq!(payload["replicas"], 1);
        assert_eq!(payload["database_instances"], 1);
        assert_eq!(payload["hostname"], format!("acme-login.{ZONE}"));
    })
    .await;
}

/// @spec-free-5
#[tokio::test]
async fn a_second_free_deployment_is_refused_with_the_instance_limit() {
    scenario(async |installation| {
        installation.serve_region().await;
        let app = installation.app();
        let path = installation.deployments(installation.free_organisation);

        let (status, first) = call(
            &app,
            &path,
            installation.command("first-tenant", "ferriskey", "free"),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{first}");

        let (status, second) = call(
            &app,
            &path,
            installation.command("second-tenant", "ferriskey", "free"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{second}");
        assert!(
            second.to_string().contains("instances"),
            "the refusal names the limit: {second}"
        );

        let recorded: i64 =
            sqlx::query_scalar("SELECT count(*) FROM deployments WHERE organisation_id = $1")
                .bind(installation.free_organisation)
                .fetch_one(&installation.pool)
                .await
                .expect("count");
        assert_eq!(recorded, 1);
    })
    .await;
}

/// @spec-free-6
#[tokio::test]
async fn a_starter_organisation_keeps_a_database_of_its_own_on_the_standard_offer() {
    scenario(async |installation| {
        installation.serve_region().await;

        let (status, created) = call(
            &installation.app(),
            &installation.deployments(installation.paid_organisation),
            installation.command("paid-tenant", "ferriskey", "standard"),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");

        let id = created["data"]["id"].as_str().expect("an id");
        let actions = installation.actions_of(id).await;
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(actions[0].0, "deployment.create");
        assert_eq!(actions[0].1["database"], "managed");
    })
    .await;
}

#[tokio::test]
async fn a_keycloak_deployment_cannot_take_the_free_offer() {
    scenario(async |installation| {
        let (status, refused) = call(
            &installation.app(),
            &installation.deployments(installation.free_organisation),
            installation.command("kc-tenant", "keycloak", "free"),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    })
    .await;
}

#[tokio::test]
async fn a_free_organisation_is_refused_the_sandbox_and_told_which_tier_opens_it() {
    scenario(async |installation| {
        installation.serve_region().await;

        let (status, refused) = call(
            &installation.app(),
            &installation.deployments(installation.free_organisation),
            installation.command("sandboxed", "ferriskey", "sandbox"),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert!(refused.to_string().contains("starter"), "{refused}");
    })
    .await;
}
