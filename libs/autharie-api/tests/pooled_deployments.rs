use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    handlers::deployments::{
        backups::ask_for_backup_handler, create_deployment::create_deployment_handler,
        delete_deployment::delete_deployment_handler, get_deployment::get_deployment_handler,
        network_access::set_network_access_handler, update_deployment::update_deployment_handler,
        upgrade_deployment::upgrade_deployment_handler,
    },
    state::AppState,
};
use autharie_auth::{Client, Identity, User};
use autharie_core::{
    AutharieService,
    cells::CreateCellCommand,
    dataplane::{
        ports::DataPlaneService,
        value_objects::{Capacity, CreateDataplaneCommand, DataPlaneAllocation, Region},
    },
    deployments::{
        DeploymentId, DeploymentKind, DeploymentName,
        commands::{CreateDeploymentCommand, DeploymentOutcome, ReportDeploymentOutcomeCommand},
        environment::Environment,
        ports::DeploymentService,
    },
    offers::Offer,
    organisation::OrganisationId,
    user::UserId,
    version::Version,
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

struct Installation {
    pool: PgPool,
    operator: Uuid,
    cells_organisation: Uuid,
    tenant: Uuid,
    owner: Uuid,
    region: String,
    release: String,
}

async fn a_user(pool: &PgPool, id: Uuid) {
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'pooled', $3)")
        .bind(id)
        .bind(format!("{id}@pooled-api.test"))
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
         VALUES ($1, 'pooled', $2, $3, 'active', $4, $5, 2, 1, now(), now())",
    )
    .bind(id)
    .bind(format!("pooled-api-{id}"))
    .bind(owner)
    .bind(plan)
    .bind(max_instances)
    .execute(pool)
    .await
    .expect("an organisation");
}

impl Installation {
    async fn seed(pool: PgPool) -> Self {
        let installation = Self {
            pool,
            operator: Uuid::new_v4(),
            cells_organisation: Uuid::new_v4(),
            tenant: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            region: format!("pooled-api-{}", Uuid::new_v4()),
            release: format!("0.0.{}", Uuid::new_v4().as_u128() as u32 % 1_000_000),
        };
        let pool = &installation.pool;

        a_user(pool, installation.operator).await;
        a_user(pool, installation.owner).await;
        an_organisation(
            pool,
            installation.cells_organisation,
            installation.operator,
            "enterprise",
            100,
        )
        .await;
        an_organisation(pool, installation.tenant, installation.owner, "free", 1).await;
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
            .with_cells_organisation(Some(OrganisationId(self.cells_organisation)))
            .with_domain(Some(ZONE.to_string()))
            .with_platform_labels(vec!["control".to_string()])
    }

    async fn serve_region(&self) -> Uuid {
        self.serve(&self.region).await
    }

    async fn serve(&self, region: &str) -> Uuid {
        let service = self.service();
        service
            .ensure_first_operator(&self.operator.to_string())
            .await
            .expect("the operator holds every right");
        let registered = service
            .create_dataplane(
                user(self.operator),
                CreateDataplaneCommand {
                    region: Region::new(region.to_string()),
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

    async fn open_a_cell(&self) -> DeploymentId {
        self.open_a_cell_in(&self.region).await
    }

    async fn open_a_cell_in(&self, region: &str) -> DeploymentId {
        let service = self.service();
        let cell = service
            .create_cell(
                user(self.operator),
                CreateCellCommand::new(Region::new(region.to_string()), Some(20)).expect("a cell"),
            )
            .await
            .expect("a cell is created");
        self.report(cell.instance(), DeploymentOutcome::Running)
            .await;
        cell.instance()
    }

    async fn report(&self, deployment: DeploymentId, outcome: DeploymentOutcome) {
        self.report_with(deployment, outcome, None).await
    }

    async fn report_with(
        &self,
        deployment: DeploymentId,
        outcome: DeploymentOutcome,
        reason: Option<&str>,
    ) {
        let service = self.service();
        let dataplane_id = service
            .get_deployment(deployment)
            .await
            .expect("read")
            .expect("the deployment exists")
            .dataplane_id;
        let subject: String =
            sqlx::query_scalar("SELECT herald_subject FROM data_planes WHERE id = $1")
                .bind(dataplane_id.0)
                .fetch_one(&self.pool)
                .await
                .expect("the herald of the plane");

        service
            .report_outcome(
                herald(subject.parse().expect("a uuid")),
                ReportDeploymentOutcomeCommand {
                    observed_version: None,
                    dataplane_id,
                    deployment_id: deployment,
                    outcome,
                    reason: reason.map(str::to_string),
                },
            )
            .await
            .expect("the outcome was recorded");
    }

    fn app(&self) -> Router {
        let mut args = Args::default();
        args.dataplane.default_region = self.region.clone();

        Router::new()
            .typed_post(create_deployment_handler)
            .typed_get(get_deployment_handler)
            .typed_delete(delete_deployment_handler)
            .typed_patch(update_deployment_handler)
            .typed_put(set_network_access_handler)
            .typed_post(upgrade_deployment_handler)
            .typed_post(ask_for_backup_handler)
            .layer(Extension(user(self.owner)))
            .with_state(AppState {
                args: Arc::new(args),
                service: self.service(),
                certificate_source: None,
                quickwit_search: None,
                quickwit_traces: None,
            })
    }

    fn deployments(&self) -> String {
        format!("/organisations/{}/deployments", self.tenant)
    }

    fn free(&self, name: &str, kind: &str) -> Value {
        json!({
            "name": name,
            "kind": kind,
            "version": self.release,
            "environment": "production",
            "offer": "free",
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
        for organisation in [self.tenant, self.cells_organisation] {
            for statement in [
                "DELETE FROM actions WHERE deployment_id IN \
                 (SELECT id FROM deployments WHERE organisation_id = $1)",
                "DELETE FROM backup_schedules WHERE deployment_id IN \
                 (SELECT id FROM deployments WHERE organisation_id = $1)",
                "DELETE FROM deployments WHERE organisation_id = $1 AND cell_id IS NOT NULL",
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
        for organisation in [self.tenant, self.cells_organisation] {
            sqlx::query("DELETE FROM organisations WHERE id = $1")
                .bind(organisation)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
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
        username: "pooled".to_string(),
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

/// @spec-fpr-1 @spec-fpr-13
#[tokio::test]
async fn a_free_organisation_gets_a_realm_named_after_its_hostname_in_an_open_cell() {
    scenario(async |installation| {
        let cell_plane = installation.serve_region().await;
        installation.open_a_cell().await;
        let app = installation.app();

        let (status, created) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("Acme Login", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let deployment = &created["data"];
        assert_eq!(deployment["distribution"]["pooled"]["realm"], "acme-login");
        assert_eq!(deployment.get("dataplane_id"), None, "{deployment}");
        assert_eq!(deployment["distribution"]["pooled"].get("cell_id"), None);
        let placed_on: Uuid =
            sqlx::query_scalar("SELECT dataplane_id FROM deployments WHERE id = $1")
                .bind(
                    deployment["id"]
                        .as_str()
                        .expect("an id")
                        .parse::<Uuid>()
                        .expect("a uuid"),
                )
                .fetch_one(&installation.pool)
                .await
                .expect("the stored plane");
        assert_eq!(placed_on, cell_plane);
        assert_eq!(deployment["offer"], "free");

        let id = deployment["id"].as_str().expect("an id");
        let actions = installation.actions_of(id).await;
        assert_eq!(actions.len(), 1, "{actions:?}");
        let (action_type, payload) = &actions[0];
        assert_eq!(action_type, "realm.create");
        assert_eq!(payload["realm"], "acme-login");
        assert_eq!(payload["hostname"], format!("acme-login.{ZONE}"));

        let schedules: i64 =
            sqlx::query_scalar("SELECT count(*) FROM backup_schedules WHERE deployment_id = $1")
                .bind(id.parse::<Uuid>().expect("a uuid"))
                .fetch_one(&installation.pool)
                .await
                .expect("count");
        assert_eq!(schedules, 0, "a realm has no archive of its own");

        let targets = installation
            .service()
            .dns_targets()
            .await
            .expect("the DNS targets");
        assert!(
            targets
                .iter()
                .any(|(target, address)| target.id.0.to_string() == id && address == GATEWAY),
            "the realm hostname points at the gateway of its cell"
        );
    })
    .await;
}

#[tokio::test]
async fn a_free_organisation_holds_one_deployment() {
    scenario(async |installation| {
        installation.serve_region().await;
        installation.open_a_cell().await;
        let app = installation.app();

        let (status, first) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("first-tenant", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{first}");

        let (status, second) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("second-tenant", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{second}");
        assert!(
            second.to_string().contains("instances"),
            "the refusal names the limit: {second}"
        );
    })
    .await;
}

/// @spec-fpr-5
#[tokio::test]
async fn with_no_open_cell_the_creation_is_refused_and_nothing_is_recorded() {
    scenario(async |installation| {
        installation.serve_region().await;
        let app = installation.app();

        let (status, refused) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("no-room", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");

        let recorded: i64 =
            sqlx::query_scalar("SELECT count(*) FROM deployments WHERE organisation_id = $1")
                .bind(installation.tenant)
                .fetch_one(&installation.pool)
                .await
                .expect("count");
        assert_eq!(recorded, 0);
    })
    .await;
}

#[tokio::test]
async fn a_keycloak_deployment_cannot_take_the_free_offer() {
    scenario(async |installation| {
        let app = installation.app();

        let (status, refused) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("kc-tenant", "keycloak")),
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
        let app = installation.app();
        let mut sandbox = installation.free("sandboxed", "ferriskey");
        sandbox["offer"] = json!("sandbox");

        let (status, refused) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(sandbox),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert!(refused.to_string().contains("starter"), "{refused}");
    })
    .await;
}

/// @spec-fpr-8
#[tokio::test]
async fn deleting_a_free_deployment_asks_its_cell_to_delete_the_realm_and_frees_the_name_once_done()
{
    scenario(async |installation| {
        installation.serve_region().await;
        installation.open_a_cell().await;
        let app = installation.app();

        let (status, created) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("leaving", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().expect("an id").to_string();

        let (status, deleted) = call(
            &app,
            Method::DELETE,
            &format!("{}/{id}", installation.deployments()),
            None,
        )
        .await;
        assert!(status.is_success(), "{status} {deleted}");
        let kinds: Vec<String> = installation
            .actions_of(&id)
            .await
            .into_iter()
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(kinds, ["realm.create", "realm.delete"]);

        let (status, refused) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("leaving", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert!(
            refused.to_string().contains("still being released"),
            "{refused}"
        );

        installation
            .report(
                DeploymentId(id.parse().expect("a uuid")),
                DeploymentOutcome::Deleted,
            )
            .await;
        let (status, again) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("leaving", "ferriskey")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{again}");
    })
    .await;
}

#[tokio::test]
async fn a_free_deployment_cannot_be_changed_through_the_update_route() {
    scenario(async |installation| {
        installation.serve_region().await;
        installation.open_a_cell().await;
        let app = installation.app();

        let (_, created) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("fixed-name", "ferriskey")),
        )
        .await;
        let id = created["data"]["id"].as_str().expect("an id").to_string();

        let (status, refused) = call(
            &app,
            Method::PATCH,
            &format!("{}/{id}", installation.deployments()),
            Some(json!({"name": "another-name"})),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert!(
            refused.to_string().contains("its name is its realm"),
            "{refused}"
        );

        for change in [
            json!({"kind": "keycloak"}),
            json!({"status": "successful"}),
            json!({"namespace": "somewhere-else"}),
            json!({"version": installation.release}),
        ] {
            let (status, refused) = call(
                &app,
                Method::PATCH,
                &format!("{}/{id}", installation.deployments()),
                Some(change.clone()),
            )
            .await;

            assert_eq!(status, StatusCode::CONFLICT, "{change}: {refused}");
            assert!(
                refused.to_string().contains("realm in a shared cell"),
                "{change}: {refused}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_free_deployment_refuses_what_only_a_dedicated_instance_can_do() {
    scenario(async |installation| {
        installation.serve_region().await;
        installation.open_a_cell().await;
        let app = installation.app();

        let (_, created) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("no-extras", "ferriskey")),
        )
        .await;
        let deployment = format!(
            "{}/{}",
            installation.deployments(),
            created["data"]["id"].as_str().expect("an id")
        );

        for (method, path, body) in [
            (
                Method::PUT,
                format!("{deployment}/network-access"),
                Some(json!({"allowed_cidrs": ["10.0.0.0/8"]})),
            ),
            (
                Method::POST,
                format!("{deployment}/upgrade"),
                Some(json!({"version": installation.release})),
            ),
            (Method::POST, format!("{deployment}/backups"), None),
            (
                Method::PATCH,
                deployment.clone(),
                Some(json!({"log_shipping_enabled": true})),
            ),
        ] {
            let (status, refused) = call(&app, method.clone(), &path, body).await;

            assert_eq!(status, StatusCode::CONFLICT, "{method} {path}: {refused}");
            assert!(
                refused.to_string().contains("realm in a shared cell"),
                "{method} {path}: {refused}"
            );
        }
    })
    .await;
}

/// @spec-fpr-7
#[tokio::test]
async fn a_free_deployment_shows_its_tenant_url_its_issuer_and_why_its_realm_was_refused() {
    scenario(async |installation| {
        installation.serve_region().await;
        installation.open_a_cell().await;
        let app = installation.app();

        let (_, created) = call(
            &app,
            Method::POST,
            &installation.deployments(),
            Some(installation.free("shown", "ferriskey")),
        )
        .await;
        let id = created["data"]["id"].as_str().expect("an id").to_string();
        let path = format!("{}/{id}", installation.deployments());

        let (status, read) = call(&app, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert_eq!(read["data"].get("dataplane_id"), None, "{read}");
        assert_eq!(read["data"]["distribution"]["pooled"].get("cell_id"), None);
        assert_eq!(read["realm"]["name"], "shown");
        assert_eq!(read["realm"]["url"], format!("https://shown.{ZONE}"));
        assert_eq!(
            read["realm"]["issuer"],
            format!("https://shown.{ZONE}/api/realms/shown")
        );
        assert_eq!(read["realm"]["refusal"], Value::Null);

        installation
            .report_with(
                DeploymentId(id.parse().expect("a uuid")),
                DeploymentOutcome::Failed,
                Some("the realm name was refused by the cell"),
            )
            .await;

        let (_, refused) = call(&app, Method::GET, &path, None).await;
        assert_eq!(
            refused["realm"]["refusal"],
            "the realm name was refused by the cell"
        );
    })
    .await;
}

#[tokio::test]
async fn two_free_deployments_asked_at_once_in_two_regions_leave_one() {
    scenario(async |installation| {
        let elsewhere = format!("{}-b", installation.region);
        installation.serve_region().await;
        installation.serve(&elsewhere).await;
        installation.open_a_cell().await;
        installation.open_a_cell_in(&elsewhere).await;

        let create = |name: &str, region: &str| {
            let service = installation.service();
            let command = CreateDeploymentCommand::new(
                OrganisationId(installation.tenant),
                DeploymentName(name.to_string()),
                DeploymentKind::Ferriskey,
                Version::parse(&installation.release).expect("a version"),
                UserId(installation.owner),
                Environment::Production,
                Region::new(region.to_string()),
                Offer::Free,
            )
            .expect("a creatable name");
            let caller = user(installation.owner);
            async move { service.create_deployment(caller, command).await }
        };

        let (first, second) = tokio::join!(
            create("racing-one", &installation.region),
            create("racing-two", &elsewhere)
        );

        let outcomes = [first, second];
        let created = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        let refused: Vec<String> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().err().map(ToString::to_string))
            .collect();
        assert_eq!(created, 1, "{refused:?}");
        assert!(refused[0].contains("instances"), "{refused:?}");
    })
    .await;
}

#[tokio::test]
async fn a_free_deployment_waits_for_the_count_of_its_organisation() {
    scenario(async |installation| {
        installation.serve_region().await;
        installation.open_a_cell().await;

        let mut holder = installation.pool.begin().await.expect("a transaction");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("org-instances:{}", installation.tenant))
            .execute(&mut *holder)
            .await
            .expect("the organisation count is held");

        let service = installation.service();
        let command = CreateDeploymentCommand::new(
            OrganisationId(installation.tenant),
            DeploymentName("waiting".to_string()),
            DeploymentKind::Ferriskey,
            Version::parse(&installation.release).expect("a version"),
            UserId(installation.owner),
            Environment::Production,
            Region::new(installation.region.clone()),
            Offer::Free,
        )
        .expect("a creatable name");
        let caller = user(installation.owner);
        let creation =
            tokio::spawn(async move { service.create_deployment(caller, command).await });

        let mut waited = false;
        for _ in 0..100 {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
                 AND objid = (hashtext($1)::bigint & 4294967295)::oid",
            )
            .bind(format!("org-instances:{}", installation.tenant))
            .fetch_one(&installation.pool)
            .await
            .expect("the locks");
            if waiting > 0 {
                waited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            waited,
            "the creation never waited for the organisation count"
        );
        assert!(!creation.is_finished());

        holder.rollback().await.expect("released");
        creation
            .await
            .expect("joined")
            .expect("created once the count is free");
    })
    .await;
}

#[tokio::test]
async fn a_free_deployment_cannot_take_a_platform_or_cell_hostname() {
    scenario(async |installation| {
        installation.serve_region().await;
        let instance = installation.open_a_cell().await;
        let instance_slug: String = sqlx::query_scalar(
            "UPDATE deployments SET deleted_at = now() WHERE id = $1 RETURNING hostname_slug",
        )
        .bind(instance.0)
        .fetch_one(&installation.pool)
        .await
        .expect("the cell instance hostname is no longer live");
        let app = installation.app();

        for name in ["control", "api-tenant", "support", instance_slug.as_str()] {
            let (status, refused) = call(
                &app,
                Method::POST,
                &installation.deployments(),
                Some(installation.free(name, "ferriskey")),
            )
            .await;

            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{name}: {refused}"
            );
            assert!(
                refused.to_string().contains("reserved"),
                "{name}: {refused}"
            );
        }
    })
    .await;
}
