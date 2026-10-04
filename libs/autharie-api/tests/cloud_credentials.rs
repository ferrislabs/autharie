use std::{
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use autharie_api::{
    args::Args,
    handlers::{
        cloud_credentials::cloud_credential_router,
        deployments::create_deployment::create_deployment_handler,
    },
    state::AppState,
};
use autharie_auth::{Identity, User};
use autharie_core::{
    AutharieService, CloudProviders, FixedCloudProvider, FixedVerdict,
    dataplane::cloud_provider::{
        ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeOffer, NodeType,
        ProviderOffers,
    },
};
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
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

const SECRET: &str = "SCW-LEAK-CANARY-0123456789abcdef";
const KEY_32_A: &str = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE=";

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

fn offers() -> ProviderOffers {
    ProviderOffers {
        control_planes: vec![
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("mutualized"),
                kind: ControlPlaneKind::Mutualized,
                monthly_price: Money::ZERO,
            },
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("dedicated-4"),
                kind: ControlPlaneKind::Dedicated,
                monthly_price: Money::new(7_000),
            },
        ],
        node_types: vec![NodeOffer {
            node_type: NodeType::new("small"),
            monthly_price: Money::new(1_000),
        }],
    }
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

fn state(pool: PgPool, verdict: FixedVerdict, keys: TransitKeyProvider) -> AppState {
    let service = AutharieService::new(pool)
        .with_cloud_providers(CloudProviders::Fixed(FixedCloudProvider {
            offers: offers(),
            verdict,
        }))
        .with_credential_keys(Some(keys));

    AppState {
        args: Arc::new(Args::default()),
        service,
        certificate_source: None,
        quickwit_search: None,
        quickwit_traces: None,
    }
}

struct Tenant {
    organisation: Uuid,
    owner: Uuid,
}

async fn tenant(pool: &PgPool) -> Tenant {
    let owner = Uuid::new_v4();
    let organisation = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'cloud', $3)")
        .bind(owner)
        .bind(format!("{owner}@cloud-api.test"))
        .bind(owner.to_string())
        .execute(pool)
        .await
        .expect("a user");
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'cloud', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
    )
    .bind(organisation)
    .bind(format!("cloud-api-{organisation}"))
    .bind(owner)
    .execute(pool)
    .await
    .expect("an organisation");

    Tenant {
        organisation,
        owner,
    }
}

async fn forget(pool: &PgPool, tenant: &Tenant) {
    for statement in [
        "DELETE FROM cluster_inventory WHERE data_plane_id IN \
         (SELECT id FROM data_planes WHERE organisation_id = $1)",
        "DELETE FROM data_planes WHERE organisation_id = $1",
        "DELETE FROM cloud_credentials WHERE organisation_id = $1",
        "DELETE FROM cloud_credential_secrets WHERE organisation_id = $1",
        "DELETE FROM organisations WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(tenant.organisation)
            .execute(pool)
            .await
            .expect("cleanup");
    }
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(tenant.owner)
        .execute(pool)
        .await
        .expect("cleanup");
}

fn identity(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "owner".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

fn app(state: AppState, sub: Uuid) -> Router {
    cloud_credential_router()
        .typed_post(create_deployment_handler)
        .layer(Extension(identity(sub)))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
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
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };

    (status, value)
}

fn register_body() -> Value {
    json!({"provider": "scaleway", "label": "production", "secret": SECRET})
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

fn captured_logs() -> Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();

    CAPTURE
        .get_or_init(|| {
            let capture = Capture::default();
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_writer(capture.clone())
                .finish();
            tracing::subscriber::set_global_default(subscriber).expect("one global subscriber");
            capture
        })
        .clone()
}

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

#[tokio::test]
async fn a_registered_credential_is_returned_without_its_secret_spec_ccp_1() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let tenant = tenant(&pool).await;
    let app = app(
        state(pool.clone(), FixedVerdict::Accept, transit(&server)),
        tenant.owner,
    );
    let base = format!("/organisations/{}/cloud-credentials", tenant.organisation);

    let (status, created) = call(&app, Method::POST, &base, Some(register_body())).await;
    let (listed_status, listed) = call(&app, Method::GET, &base, None).await;

    assert_eq!(status, StatusCode::CREATED, "{created}");
    let mut keys: Vec<_> = created
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["created_at", "id", "label", "provider", "scope_checked_at"]
    );
    assert_eq!(created["provider"], "scaleway");
    assert_eq!(created["label"], "production");
    assert_eq!(listed_status, StatusCode::OK);
    assert_eq!(listed.as_array().expect("an array").len(), 1);
    assert_eq!(listed[0], created);
    assert!(!created.to_string().contains(SECRET));
    assert!(!listed.to_string().contains(SECRET));

    forget(&pool, &tenant).await;
}

#[tokio::test]
async fn a_credential_with_missing_or_excess_permissions_is_refused_with_the_list_spec_ccp_2() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let tenant = tenant(&pool).await;
    let base = format!("/organisations/{}/cloud-credentials", tenant.organisation);

    let missing = app(
        state(
            pool.clone(),
            FixedVerdict::Missing(vec!["KubernetesFullAccess".to_string()]),
            transit(&server),
        ),
        tenant.owner,
    );
    let excess = app(
        state(
            pool.clone(),
            FixedVerdict::Excess(vec!["IAMManager".to_string()]),
            transit(&server),
        ),
        tenant.owner,
    );

    let (missing_status, missing_body) =
        call(&missing, Method::POST, &base, Some(register_body())).await;
    let (excess_status, excess_body) =
        call(&excess, Method::POST, &base, Some(register_body())).await;
    let (_, listed) = call(&missing, Method::GET, &base, None).await;

    assert_eq!(missing_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        missing_body["message"]
            .as_str()
            .expect("a message")
            .contains("KubernetesFullAccess")
    );
    assert_eq!(excess_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        excess_body["message"]
            .as_str()
            .expect("a message")
            .contains("IAMManager")
    );
    assert!(listed.as_array().expect("an array").is_empty());

    forget(&pool, &tenant).await;
}

#[tokio::test]
async fn the_secret_is_in_no_response_log_or_table_in_clear_spec_ccp_3() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let tenant = tenant(&pool).await;
    let capture = captured_logs();

    let accepting = app(
        state(pool.clone(), FixedVerdict::Accept, transit(&server)),
        tenant.owner,
    );
    let refusing = app(
        state(pool.clone(), FixedVerdict::Invalid, transit(&server)),
        tenant.owner,
    );
    let base = format!("/organisations/{}/cloud-credentials", tenant.organisation);

    let (_, accepted) = call(&accepting, Method::POST, &base, Some(register_body())).await;
    let (_, refused) = call(&refusing, Method::POST, &base, Some(register_body())).await;
    let (_, listed) = call(&accepting, Method::GET, &base, None).await;

    let responses = format!("{accepted} {refused} {listed}");
    assert!(!responses.contains(SECRET));

    let logs = String::from_utf8_lossy(&capture.0.lock().expect("lock")).to_string();
    assert!(
        !logs.is_empty(),
        "the capture saw nothing, so it proves nothing"
    );
    assert!(!logs.contains(SECRET), "the secret reached a log");

    let leaked_in_cipher: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM cloud_credential_secrets \
         WHERE position(convert_to($1, 'UTF8') in ciphertext) > 0 \
            OR wrapped_dek LIKE '%' || $1 || '%')",
    )
    .bind(SECRET)
    .fetch_one(&pool)
    .await
    .expect("a scan");
    assert!(!leaked_in_cipher, "the secret is in the credential table");

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
    )
    .fetch_all(&pool)
    .await
    .expect("tables");
    for table in tables {
        let leaked: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM \"{table}\" t WHERE t::text LIKE '%' || $1 || '%')"
        ))
        .bind(SECRET)
        .fetch_one(&pool)
        .await
        .expect("a scan");
        assert!(!leaked, "the secret is in the clear in {table}");
    }

    forget(&pool, &tenant).await;
}

#[tokio::test]
async fn a_credential_used_by_a_cluster_cannot_be_deleted_spec_ccp_4() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let tenant = tenant(&pool).await;
    let app = app(
        state(pool.clone(), FixedVerdict::Accept, transit(&server)),
        tenant.owner,
    );
    let base = format!("/organisations/{}/cloud-credentials", tenant.organisation);
    let (_, created) = call(&app, Method::POST, &base, Some(register_body())).await;
    let credential: Uuid = created["id"]
        .as_str()
        .expect("an id")
        .parse()
        .expect("a uuid");
    let plane = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO data_planes (id, mode, region, status, organisation_id, \
         capacity_cpu_millis, capacity_memory_mib, capacity_storage_gib, \
         deployment_id, credential_id) \
         VALUES ($1, 'customer', 'fr-par', 'provisioning', $2, 1, 1, 1, $3, $4)",
    )
    .bind(plane)
    .bind(tenant.organisation)
    .bind(Uuid::new_v4())
    .bind(credential)
    .execute(&pool)
    .await
    .expect("a plane");
    sqlx::query(
        "INSERT INTO cluster_inventory (data_plane_id, kind, provider_id) \
         VALUES ($1, 'cluster', 'cl-1')",
    )
    .bind(plane)
    .execute(&pool)
    .await
    .expect("a resource");

    let uri = format!("{base}/{credential}");
    let (in_use, _) = call(&app, Method::DELETE, &uri, None).await;

    sqlx::query("UPDATE cluster_inventory SET released_at = now() WHERE data_plane_id = $1")
        .bind(plane)
        .execute(&pool)
        .await
        .expect("released");
    let (deleted, _) = call(&app, Method::DELETE, &uri, None).await;
    let (again, _) = call(&app, Method::DELETE, &uri, None).await;

    assert_eq!(in_use, StatusCode::CONFLICT);
    assert_eq!(deleted, StatusCode::NO_CONTENT);
    assert_eq!(again, StatusCode::NOT_FOUND);

    forget(&pool, &tenant).await;
}

#[tokio::test]
async fn offers_and_the_estimated_cost_are_shown_before_creation_spec_ccp_8() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let tenant = tenant(&pool).await;
    let app = app(
        state(pool.clone(), FixedVerdict::Accept, transit(&server)),
        tenant.owner,
    );
    let base = format!("/organisations/{}", tenant.organisation);
    let (_, created) = call(
        &app,
        Method::POST,
        &format!("{base}/cloud-credentials"),
        Some(register_body()),
    )
    .await;
    let credential = created["id"].as_str().expect("an id").to_string();

    let (offers_status, listed) = call(
        &app,
        Method::GET,
        &format!("{base}/cloud-credentials/{credential}/offers?region=fr-par"),
        None,
    )
    .await;
    let estimate =
        |mode: &str, control_plane: &str, node: &str, min: u8, max: u8, replication: u8| {
            json!({
                "credential_id": credential, "region": "fr-par", "mode": mode,
                "control_plane_id": control_plane, "node_type": node,
                "min_nodes": min, "max_nodes": max, "replication": replication
            })
        };
    let (ok_status, priced) = call(
        &app,
        Method::POST,
        &format!("{base}/cluster-profiles/estimate"),
        Some(estimate("ha", "dedicated-4", "small", 3, 10, 2)),
    )
    .await;
    let (refused_status, refused) = call(
        &app,
        Method::POST,
        &format!("{base}/cluster-profiles/estimate"),
        Some(estimate("dev", "mutualized", "small", 2, 2, 1)),
    )
    .await;
    let (unknown_status, _) = call(
        &app,
        Method::POST,
        &format!("{base}/cluster-profiles/estimate"),
        Some(estimate("standard", "mutualized", "gigantic", 2, 4, 2)),
    )
    .await;

    assert_eq!(offers_status, StatusCode::OK);
    assert_eq!(
        listed,
        json!({
            "control_planes": [
                {"id": "mutualized", "kind": "mutualized", "monthly_price": 0},
                {"id": "dedicated-4", "kind": "dedicated", "monthly_price": 7000}
            ],
            "node_types": [{"node_type": "small", "monthly_price": 1000}]
        })
    );
    assert_eq!(ok_status, StatusCode::OK);
    assert_eq!(priced, json!({"min": 10_000, "max": 17_000}));
    assert_eq!(refused_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        refused["message"]
            .as_str()
            .expect("a message")
            .contains("dev")
    );
    assert_eq!(unknown_status, StatusCode::UNPROCESSABLE_ENTITY);

    forget(&pool, &tenant).await;
}

#[tokio::test]
async fn another_organisations_credential_is_not_found_and_a_stranger_is_forbidden() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let mine = tenant(&pool).await;
    let theirs = tenant(&pool).await;
    let state = state(pool.clone(), FixedVerdict::Accept, transit(&server));
    let owner = app(state.clone(), mine.owner);
    let stranger = app(state, theirs.owner);
    let base = format!("/organisations/{}/cloud-credentials", mine.organisation);
    let (_, created) = call(&owner, Method::POST, &base, Some(register_body())).await;
    let credential = created["id"].as_str().expect("an id").to_string();

    let (forbidden_list, _) = call(&stranger, Method::GET, &base, None).await;
    let (forbidden_register, _) = call(&stranger, Method::POST, &base, Some(register_body())).await;
    let (wrong_organisation, _) = call(
        &stranger,
        Method::GET,
        &format!(
            "/organisations/{}/cloud-credentials/{credential}/offers?region=fr-par",
            theirs.organisation
        ),
        None,
    )
    .await;

    assert_eq!(forbidden_list, StatusCode::FORBIDDEN);
    assert_eq!(forbidden_register, StatusCode::FORBIDDEN);
    assert_eq!(wrong_organisation, StatusCode::NOT_FOUND);

    forget(&pool, &mine).await;
    forget(&pool, &theirs).await;
}

#[tokio::test]
async fn a_keycloak_deployment_cannot_use_the_customer_cloud_spec_ccp_9() {
    let Some(pool) = pool().await else { return };
    let _ = captured_logs();
    let server = MockServer::start_async().await;
    let tenant = tenant(&pool).await;
    let app = app(
        state(pool.clone(), FixedVerdict::Accept, transit(&server)),
        tenant.owner,
    );

    let (status, body) = call(
        &app,
        Method::POST,
        &format!("/organisations/{}/deployments", tenant.organisation),
        Some(json!({
            "name": "app", "kind": "keycloak", "version": "1.0.0",
            "environment": "production", "offer": "standard",
            "distribution": {
                "type": "customer_cloud",
                "credential_id": Uuid::new_v4(),
                "region": "fr-par",
                "profile": {
                    "mode": "dev", "control_plane_id": "mutualized", "node_type": "small",
                    "min_nodes": 1, "max_nodes": 1, "replication": 1
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    forget(&pool, &tenant).await;
}
