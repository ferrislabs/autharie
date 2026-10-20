use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::platform::cells::{
        create_cell_handler, drain_cell_handler, get_cell_handler, get_cells_settings_handler,
        list_cells_handler, retire_cell_handler, update_cells_settings_handler,
    },
    state::AppState,
};
use autharie_auth::{Client, Identity, User};
use autharie_core::{
    AutharieService,
    cells::{CELL_DATABASE_INSTANCE, cell_reservation},
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
};
use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use axum_extra::routing::RouterExt;
use futures::FutureExt;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, postgres::PgPoolOptions};
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

const SETTINGS_LOCK: i64 = 20_261_020;

struct Installation {
    _settings_lock: PgConnection,
    pool: PgPool,
    operator: Uuid,
    organisation: Uuid,
    region: String,
    release: String,
    herald: Uuid,
}

impl Installation {
    async fn seed(pool: PgPool) -> Self {
        let mut settings_lock = pool.acquire().await.expect("a connection").detach();
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(SETTINGS_LOCK)
            .execute(&mut settings_lock)
            .await
            .expect("the cells settings are ours");
        let operator = Uuid::new_v4();
        let organisation = Uuid::new_v4();
        let release = format!("0.9.{}", rand_patch());

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

        sqlx::query("UPDATE platform_cells_settings SET organisation_id = $1")
            .bind(organisation)
            .execute(&pool)
            .await
            .expect("the cells organisation is chosen");

        Self {
            _settings_lock: settings_lock,
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
        .typed_get(get_cells_settings_handler)
        .typed_put(update_cells_settings_handler)
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
                reason: None,
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
        assert_eq!(
            deployment.resources,
            cell_reservation(Topology::HIGH_AVAILABILITY)
        );

        let payload = create_deployment_payload(&installation.pool, instance).await;
        assert_eq!(payload["replicas"], 2);
        assert_eq!(payload["database_instances"], 2);
        assert_eq!(payload["pooled_cell"], true);
        assert_eq!(payload["cpu_millis"], CELL_DATABASE_INSTANCE.cpu_millis);
        assert_eq!(payload["memory_mib"], CELL_DATABASE_INSTANCE.memory_mib);
        assert_eq!(payload["storage_gib"], CELL_DATABASE_INSTANCE.storage_gib);

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
async fn without_a_cells_organisation_creating_a_cell_is_unavailable_until_one_is_chosen() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool.clone()).await;
    installation.serve_region().await;
    sqlx::query("UPDATE platform_cells_settings SET organisation_id = NULL")
        .execute(&pool)
        .await
        .expect("no cells organisation");
    let app = app(installation.service(), operator(installation.operator));
    let create = || {
        call(
            &app,
            Method::POST,
            "/platform/cells",
            Some(json!({"region": installation.region})),
        )
    };

    let (status, refused) = create().await;
    let (listed, _) = call(&app, Method::GET, "/platform/cells", None).await;
    let (read_status, read) = call(&app, Method::GET, "/platform/cells/settings", None).await;
    let (saved_status, saved) = call(
        &app,
        Method::PUT,
        "/platform/cells/settings",
        Some(json!({"organisation_id": installation.organisation})),
    )
    .await;
    let (created_status, created) = create().await;

    installation.forget().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert!(
        refused.to_string().contains("choose the organisation"),
        "{refused}"
    );
    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read_status, StatusCode::OK, "{read}");
    assert_eq!(read["organisation_id"], Value::Null);
    assert_eq!(saved_status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["organisation_id"],
        installation.organisation.to_string()
    );
    assert_eq!(saved["updated_by"], installation.operator.to_string());
    assert_eq!(created_status, StatusCode::CREATED, "{created}");
}

#[tokio::test]
async fn the_cells_organisation_changes_only_while_every_cell_is_retired() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool.clone()).await;
    installation.serve_region().await;
    let app = app(installation.service(), operator(installation.operator));
    let other = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'cells', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
    )
    .bind(other)
    .bind(format!("cells-other-{other}"))
    .bind(installation.operator)
    .execute(&pool)
    .await
    .expect("another organisation");
    let put = |organisation: Value| {
        call(
            &app,
            Method::PUT,
            "/platform/cells/settings",
            Some(json!({ "organisation_id": organisation })),
        )
    };

    let (cell_id, _) = create_a_cell(&app, &installation.region).await;
    let (held, held_body) = put(json!(other)).await;
    let (cleared_held, _) = put(Value::Null).await;
    let (unchanged, unchanged_body) = put(json!(installation.organisation)).await;
    let (unknown, _) = put(json!(Uuid::new_v4())).await;
    let (retired, _) = call(
        &app,
        Method::POST,
        &format!("/platform/cells/{cell_id}/retire"),
        None,
    )
    .await;
    let (moved, moved_body) = put(json!(other)).await;
    let (_, read) = call(&app, Method::GET, "/platform/cells/settings", None).await;
    let (cleared, cleared_body) = put(Value::Null).await;

    sqlx::query("DELETE FROM organisations WHERE id = $1")
        .bind(other)
        .execute(&pool)
        .await
        .expect("cleanup");
    installation.forget().await;
    assert_eq!(held, StatusCode::CONFLICT, "{held_body}");
    assert_eq!(cleared_held, StatusCode::CONFLICT);
    assert_eq!(unchanged, StatusCode::OK, "{unchanged_body}");
    assert_eq!(unknown, StatusCode::NOT_FOUND);
    assert_eq!(retired, StatusCode::OK);
    assert_eq!(moved, StatusCode::OK, "{moved_body}");
    assert_eq!(read["organisation_id"], other.to_string());
    assert_eq!(cleared, StatusCode::OK, "{cleared_body}");
    assert_eq!(cleared_body["organisation_id"], Value::Null);
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

async fn create_a_cell(app: &Router, region: &str) -> (String, Uuid) {
    let (status, created) = call(
        app,
        Method::POST,
        "/platform/cells",
        Some(json!({"region": region})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    (
        created["data"]["id"].as_str().expect("an id").to_string(),
        created["data"]["instance_deployment_id"]
            .as_str()
            .expect("an instance")
            .parse()
            .expect("a uuid"),
    )
}

async fn grant(pool: &PgPool, subject: Uuid, rights: &[&str]) {
    sqlx::query("INSERT INTO platform_operators (subject, rights) VALUES ($1, $2)")
        .bind(subject.to_string())
        .bind(rights)
        .execute(pool)
        .await
        .expect("a grant");
}

async fn revoke(pool: &PgPool, subject: Uuid) {
    sqlx::query("DELETE FROM platform_operators WHERE subject = $1")
        .bind(subject.to_string())
        .execute(pool)
        .await
        .expect("cleanup");
}

fn status_of(error: autharie_core::CoreError) -> StatusCode {
    use axum::response::IntoResponse;

    autharie_api::errors::ApiError::from(error)
        .into_response()
        .status()
}

#[tokio::test]
async fn the_cells_settings_need_view_estate_to_read_and_operate_fleet_to_change() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool.clone()).await;
    installation
        .service()
        .ensure_first_operator(&installation.operator.to_string())
        .await
        .expect("the operator holds every right");
    let reader = Uuid::new_v4();
    let stranger = Uuid::new_v4();
    grant(&pool, reader, &["view_estate"]).await;
    let body = Some(json!({"organisation_id": installation.organisation}));

    let mut answers = Vec::new();
    for (who, expected) in [
        (installation.operator, [StatusCode::OK, StatusCode::OK]),
        (reader, [StatusCode::OK, StatusCode::FORBIDDEN]),
        (stranger, [StatusCode::FORBIDDEN, StatusCode::FORBIDDEN]),
    ] {
        let app = app(installation.service(), operator(who));
        for (method, body, expected) in [
            (Method::GET, None, expected[0]),
            (Method::PUT, body.clone(), expected[1]),
        ] {
            let (status, _) = call(&app, method.clone(), "/platform/cells/settings", body).await;
            answers.push((who, method, status, expected));
        }
    }

    revoke(&pool, reader).await;
    installation.forget().await;
    for (who, method, status, expected) in answers {
        assert_eq!(status, expected, "{who} {method}");
    }
}

#[tokio::test]
async fn reading_the_cells_needs_view_estate_and_changing_them_needs_operate_fleet() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool.clone()).await;
    installation.serve_region().await;
    let operator_app = app(installation.service(), operator(installation.operator));
    let (cell_id, _) = create_a_cell(&operator_app, &installation.region).await;
    let reader = Uuid::new_v4();
    let stranger = Uuid::new_v4();
    grant(&pool, reader, &["view_estate"]).await;

    let routes = [
        (Method::GET, "/platform/cells".to_string(), None),
        (Method::GET, format!("/platform/cells/{cell_id}"), None),
        (
            Method::POST,
            "/platform/cells".to_string(),
            Some(json!({"region": installation.region})),
        ),
        (
            Method::POST,
            format!("/platform/cells/{cell_id}/drain"),
            None,
        ),
        (
            Method::POST,
            format!("/platform/cells/{cell_id}/retire"),
            None,
        ),
    ];
    let for_a_reader = [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::FORBIDDEN,
        StatusCode::FORBIDDEN,
        StatusCode::FORBIDDEN,
    ];

    let mut answers = Vec::new();
    for (who, expected) in [
        (reader, for_a_reader),
        (stranger, [StatusCode::FORBIDDEN; 5]),
    ] {
        let app = app(installation.service(), operator(who));
        for ((method, uri, body), expected) in routes.iter().zip(expected) {
            let (status, _) = call(&app, method.clone(), uri, body.clone()).await;
            answers.push((who == reader, method.clone(), uri.clone(), status, expected));
        }
    }

    revoke(&pool, reader).await;
    installation.forget().await;
    for (reader, method, uri, status, expected) in answers {
        assert_eq!(status, expected, "reader={reader} {method} {uri}");
    }
}

/// @spec-fpr-11
#[tokio::test]
async fn a_cell_whose_instance_serves_without_two_database_instances_stays_provisioning() {
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool.clone()).await;
    installation.serve_region().await;
    let service = installation.service();
    let app = app(service.clone(), operator(installation.operator));
    let (cell_id, instance) = create_a_cell(&app, &installation.region).await;

    sqlx::query("UPDATE deployments SET database_instances = 1 WHERE id = $1")
        .bind(instance)
        .execute(&pool)
        .await
        .expect("a topology that is not highly available");
    report(&service, &installation, instance).await;
    let (_, read) = call(
        &app,
        Method::GET,
        &format!("/platform/cells/{cell_id}"),
        None,
    )
    .await;

    installation.forget().await;
    assert_eq!(read["data"]["status"], "provisioning", "{read}");
}

#[tokio::test]
async fn the_instance_of_a_live_cell_cannot_be_deleted_or_renamed_until_the_cell_is_retired() {
    use autharie_core::deployments::{DeploymentName, commands::UpdateDeploymentCommand};

    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool).await;
    installation.serve_region().await;
    let service = installation.service();
    let app = app(service.clone(), operator(installation.operator));
    let (cell_id, instance) = create_a_cell(&app, &installation.region).await;
    let rename = || {
        UpdateDeploymentCommand::new()
            .with_name(DeploymentName("renamed-cell".to_string()))
            .expect("a name")
    };

    let deleted = service.delete_deployment(DeploymentId(instance)).await;
    let renamed = service
        .update_deployment(DeploymentId(instance), rename())
        .await;
    let still_named = service
        .get_deployment(DeploymentId(instance))
        .await
        .expect("read")
        .map(|deployment| deployment.name.0);
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/platform/cells/{cell_id}/retire"),
        None,
    )
    .await;
    let deleted_once_retired = service.delete_deployment(DeploymentId(instance)).await;

    installation.forget().await;
    for refused in [deleted.map(|_| ()), renamed.map(|_| ())] {
        let error = refused.expect_err("the instance of a live cell changed");
        assert_eq!(status_of(error), StatusCode::CONFLICT);
    }
    assert!(
        still_named.is_some_and(|name| name.starts_with("cell-")),
        "the instance kept its name"
    );
    assert_eq!(status, StatusCode::OK);
    assert!(deleted_once_retired.is_ok(), "{deleted_once_retired:?}");
}
