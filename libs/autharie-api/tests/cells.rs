use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::platform::cells::{
        create_cell_handler, drain_cell_handler, get_cell_handler, list_cells_handler,
        retire_cell_handler,
    },
    state::AppState,
};
use autharie_auth::{Client, Identity, User};
use autharie_core::{
    AutharieService,
    cells::CELL_INSTANCE_RESOURCES,
    dataplane::{
        ports::DataPlaneService,
        value_objects::{Capacity, CreateDataplaneCommand, DataPlaneAllocation, Region},
    },
    deployments::{
        DeploymentId,
        commands::{DeploymentOutcome, ReportDeploymentOutcomeCommand},
        ports::DeploymentService,
        topology::Topology,
    },
    organisation::OrganisationId,
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

struct Installation {
    pool: PgPool,
    operator: Uuid,
    organisation: Uuid,
    region: String,
    release: String,
    herald: Uuid,
}

impl Installation {
    async fn seed(pool: PgPool) -> Self {
        let operator = Uuid::new_v4();
        let organisation = Uuid::new_v4();
        let release = format!("0.0.{}", rand_patch());

        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'operator', $3)")
            .bind(operator)
            .bind(format!("{operator}@cells-api.test"))
            .bind(operator.to_string())
            .execute(&pool)
            .await
            .expect("an operator");
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'cells', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
        )
        .bind(organisation)
        .bind(format!("cells-api-{organisation}"))
        .bind(operator)
        .execute(&pool)
        .await
        .expect("the cells organisation");
        sqlx::query(
            "INSERT INTO releases (kind, version, status, risk) \
             VALUES ('ferriskey', $1, 'available', 'none')",
        )
        .bind(&release)
        .execute(&pool)
        .await
        .expect("a release");

        Self {
            pool,
            operator,
            organisation,
            region: format!("cells-api-{}", Uuid::new_v4()),
            release,
            herald: Uuid::new_v4(),
        }
    }

    fn service(&self) -> AutharieService {
        AutharieService::new(self.pool.clone())
            .with_cells_organisation(Some(OrganisationId(self.organisation)))
    }

    async fn serve_region(&self) {
        let service = self.service();
        service
            .ensure_first_operator(&self.operator.to_string())
            .await
            .expect("the operator holds every right");
        let registered = service
            .create_dataplane(
                operator(self.operator),
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
             herald_client_id = $2, herald_subject = $3 WHERE id = $1",
        )
        .bind(registered.dataplane.id.0)
        .bind(format!("herald-{}", registered.dataplane.id.0))
        .bind(self.herald.to_string())
        .execute(&self.pool)
        .await
        .expect("a data plane that reports");
    }

    async fn forget(&self) {
        let organisation = self.organisation;
        for statement in [
            "DELETE FROM actions WHERE deployment_id IN \
             (SELECT id FROM deployments WHERE organisation_id = $1)",
            "DELETE FROM cells WHERE instance_deployment_id IN \
             (SELECT id FROM deployments WHERE organisation_id = $1)",
            "DELETE FROM deployments WHERE organisation_id = $1",
        ] {
            sqlx::query(statement)
                .bind(organisation)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("DELETE FROM actions WHERE dataplane_id IN (SELECT id FROM data_planes WHERE region = $1)")
            .bind(&self.region)
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM data_planes WHERE region = $1")
            .bind(&self.region)
            .execute(&self.pool)
            .await
            .expect("cleanup");
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
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(organisation)
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(self.operator)
            .execute(&self.pool)
            .await
            .expect("cleanup");
    }
}

fn rand_patch() -> u32 {
    Uuid::new_v4().as_u128() as u32 % 1_000_000
}

fn operator(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "operator".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

fn herald(subject: Uuid) -> Identity {
    Identity::Client(Client {
        id: subject.to_string(),
        client_id: "herald".to_string(),
        roles: vec![],
        scopes: vec![],
    })
}

fn app(service: AutharieService, identity: Identity) -> Router {
    Router::new()
        .typed_post(create_cell_handler)
        .typed_get(list_cells_handler)
        .typed_get(get_cell_handler)
        .typed_post(drain_cell_handler)
        .typed_post(retire_cell_handler)
        .layer(Extension(identity))
        .with_state(AppState {
            args: Arc::new(Args::default()),
            service,
            certificate_source: None,
            quickwit_search: None,
            quickwit_traces: None,
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

async fn create_deployment_payload(pool: &PgPool, deployment: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT payload FROM actions \
         WHERE deployment_id = $1 AND action_type = 'deployment.create'",
    )
    .bind(deployment)
    .fetch_one(pool)
    .await
    .expect("a deployment.create action")
}

async fn report(service: &AutharieService, installation: &Installation, instance: Uuid) {
    let dataplane_id = service
        .get_deployment(DeploymentId(instance))
        .await
        .expect("read")
        .expect("the instance exists")
        .dataplane_id;

    service
        .report_outcome(
            herald(installation.herald),
            ReportDeploymentOutcomeCommand {
                observed_version: None,
                dataplane_id,
                deployment_id: DeploymentId(instance),
                outcome: DeploymentOutcome::Running,
            },
        )
        .await
        .expect("the outcome was recorded");
}

/// @spec-fpr-11 @spec-fpr-12
#[tokio::test]
async fn an_operator_creates_a_cell_sees_it_provisioning_then_open_then_drains_and_retires_it() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool).await;
    installation.serve_region().await;
    let service = installation.service();
    let app = app(service.clone(), operator(installation.operator));

    let outcome = async {
        let (status, created) = call(
            &app,
            Method::POST,
            "/platform/cells",
            Some(json!({"region": installation.region, "capacity": 20})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let cell = created["data"].clone();
        assert_eq!(cell["status"], "provisioning");
        assert_eq!(cell["capacity"], 20);
        assert_eq!(cell["realms"], 0);
        let cell_id = cell["id"].as_str().expect("an id").to_string();
        let instance: Uuid = cell["instance_deployment_id"]
            .as_str()
            .expect("an instance")
            .parse()
            .expect("a uuid");

        let deployment = service
            .get_deployment(DeploymentId(instance))
            .await
            .expect("read")
            .expect("the instance exists");
        assert_eq!(deployment.organisation_id.0, installation.organisation);
        assert_eq!(deployment.created_by.0, installation.operator);
        assert_eq!(deployment.topology, Topology::HIGH_AVAILABILITY);
        assert_eq!(deployment.resources, CELL_INSTANCE_RESOURCES);

        let payload = create_deployment_payload(&installation.pool, instance).await;
        assert_eq!(payload["replicas"], 2);
        assert_eq!(payload["database_instances"], 2);
        assert_eq!(payload["pooled_cell"], true);

        let (status, listed) = call(&app, Method::GET, "/platform/cells", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            listed["data"]
                .as_array()
                .expect("a list")
                .iter()
                .any(|listed| listed["id"] == cell_id.as_str()),
            "{listed}"
        );

        let (status, refused) = call(
            &app,
            Method::POST,
            &format!("/platform/cells/{cell_id}/drain"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");

        report(&service, &installation, instance).await;
        let (_, opened) = call(
            &app,
            Method::GET,
            &format!("/platform/cells/{cell_id}"),
            None,
        )
        .await;
        assert_eq!(opened["data"]["status"], "open", "{opened}");

        let (status, drained) = call(
            &app,
            Method::POST,
            &format!("/platform/cells/{cell_id}/drain"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{drained}");
        assert_eq!(drained["data"]["status"], "draining");

        let (status, retired) = call(
            &app,
            Method::POST,
            &format!("/platform/cells/{cell_id}/retire"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{retired}");
        assert_eq!(retired["data"]["status"], "retired");

        let (status, _) = call(
            &app,
            Method::GET,
            &format!("/platform/cells/{}", Uuid::new_v4()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    };
    let result = std::panic::AssertUnwindSafe(outcome).catch_unwind().await;

    installation.forget().await;
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}

#[tokio::test]
async fn without_a_cells_organisation_creating_a_cell_is_unavailable_and_says_why() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool.clone()).await;
    installation.serve_region().await;
    let app = app(AutharieService::new(pool), operator(installation.operator));

    let (status, refused) = call(
        &app,
        Method::POST,
        "/platform/cells",
        Some(json!({"region": installation.region})),
    )
    .await;
    let (listed, _) = call(&app, Method::GET, "/platform/cells", None).await;

    installation.forget().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert!(
        refused.to_string().contains("CELLS_ORGANISATION_ID"),
        "{refused}"
    );
    assert_eq!(listed, StatusCode::OK);
}

#[tokio::test]
async fn somebody_without_a_platform_right_cannot_create_a_cell() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool).await;
    let app = app(installation.service(), operator(Uuid::new_v4()));

    let (status, _) = call(
        &app,
        Method::POST,
        "/platform/cells",
        Some(json!({"region": installation.region})),
    )
    .await;

    installation.forget().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
