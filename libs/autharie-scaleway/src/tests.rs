use std::{
    io,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use autharie_domain::{
    CoreError,
    dataplane::{
        cloud_provider::{
            CatalogError, ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId,
            CredentialVerifier, Money, NodeOffer, NodeType, Provider, ProviderCatalog,
            ProviderOffers,
        },
        cluster_profile::{ClusterMode, ClusterProfile, Replication},
        credential::{CloudCredentialId, CredentialError, SecretString},
        entities::DataPlane,
        herald_identity::HeraldBinding,
        inventory::{ProvisionedResource, ResourceKind},
        provisioner::{ClusterProvisioner, ProvisionError, ProvisionRequest, ProvisionTarget},
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use httpmock::{
    Method::{DELETE, GET, PATCH, POST},
    MockServer,
    prelude::HttpMockRequest,
};
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::{
    ScalewayCatalog, ScalewayConfig, ScalewayProvisioner, ScalewayVerifier,
    fakes::{FakeBootstrapper, FakeInventory, FakeRepository, FakeStore},
};

const SECRET_KEY: &str = "SECRETKEYVALUE-0f3a";
const KUBECONFIG: &str = "apiVersion: v1\ntoken: KUBECONFIGTOKEN-77b1\n";
const GIB: u64 = 1024 * 1024 * 1024;

fn secret_json() -> String {
    format!(r#"{{"access_key":"SCWACCESSKEY","secret_key":"{SECRET_KEY}","project_id":"proj-1"}}"#)
}

fn config(server: &MockServer) -> ScalewayConfig {
    ScalewayConfig {
        base_url: server.base_url(),
        poll_interval: Duration::ZERO,
        poll_attempts: 3,
        ..ScalewayConfig::default()
    }
}

fn offers() -> ProviderOffers {
    ProviderOffers {
        control_planes: vec![
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("kapsule"),
                kind: ControlPlaneKind::Mutualized,
                monthly_price: Money::ZERO,
            },
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("kapsule-dedicated-4"),
                kind: ControlPlaneKind::Dedicated,
                monthly_price: Money::new(10_800),
            },
        ],
        node_types: vec![
            NodeOffer {
                node_type: NodeType::new("PRO2-S"),
                monthly_price: Money::new(7_200),
            },
            NodeOffer {
                node_type: NodeType::new("PRO2-M"),
                monthly_price: Money::new(14_400),
            },
        ],
    }
}

fn profile(control_plane: &str, node_type: &str) -> ClusterProfile {
    ClusterProfile::new(
        ClusterMode::Standard,
        offers()
            .control_plane(&ControlPlaneOfferId::new(control_plane))
            .cloned()
            .expect("control plane in the test catalog"),
        NodeType::new(node_type),
        2,
        4,
        Replication::new(2).expect("replication"),
        &offers(),
    )
    .expect("profile")
}

struct Harness {
    provisioner: ScalewayProvisioner<FakeStore, FakeInventory, FakeRepository, FakeBootstrapper>,
    inventory: FakeInventory,
    bootstrapper: FakeBootstrapper,
    credential_id: CloudCredentialId,
    organisation_id: OrganisationId,
    data_plane_id: DataPlaneId,
}

fn harness(server: &MockServer, data_plane: Option<DataPlane>) -> Harness {
    capture();
    let inventory = FakeInventory::default();
    let bootstrapper = FakeBootstrapper::new(inventory.clone());
    let provisioner = ScalewayProvisioner::new(
        config(server),
        FakeStore {
            secret: secret_json(),
        },
        inventory.clone(),
        FakeRepository { data_plane },
        bootstrapper.clone(),
    )
    .expect("provisioner");
    Harness {
        provisioner,
        inventory,
        bootstrapper,
        credential_id: CloudCredentialId(Uuid::new_v4()),
        organisation_id: OrganisationId(Uuid::new_v4()),
        data_plane_id: DataPlaneId(Uuid::new_v4()),
    }
}

impl Harness {
    fn request(&self, profile: ClusterProfile) -> ProvisionRequest {
        ProvisionRequest {
            data_plane_id: self.data_plane_id,
            organisation_id: self.organisation_id,
            region: Region::new("fr-par"),
            minimum: DeploymentResources::DEFAULT,
            target: ProvisionTarget::Customer {
                credential_id: self.credential_id,
                profile,
                deployment_id: DeploymentId(Uuid::new_v4()),
            },
        }
    }
}

fn customer_data_plane(id: DataPlaneId, credential_id: CloudCredentialId) -> DataPlane {
    let mut data_plane = DataPlane::new(
        DataPlaneAllocation::Customer {
            organisation_id: OrganisationId(Uuid::new_v4()),
            deployment_id: DeploymentId(Uuid::new_v4()),
            credential_id,
        },
        Region::new("fr-par"),
        Capacity::new(1000, 1024, 1).expect("capacity"),
    );
    data_plane.id = id;
    data_plane
}

fn resource(kind: ResourceKind, id: &str) -> ProvisionedResource {
    ProvisionedResource {
        kind,
        provider_id: id.to_string(),
    }
}

fn mock_servers(server: &MockServer) {
    server.mock(|when, then| {
        when.method(GET)
            .path("/instance/v1/zones/fr-par-1/products/servers")
            .header("x-auth-token", SECRET_KEY);
        then.status(200).json_body(serde_json::json!({
            "servers": {
                "PRO2-S": {"hourly_price": 0.1, "ncpus": 4, "ram": 16 * GIB},
                "PRO2-M": {"hourly_price": 0.2, "ncpus": 8, "ram": 32 * GIB},
                "DEV1-S": {"hourly_price": 0.01, "ncpus": 2, "ram": 2 * GIB}
            }
        }));
    });
}

fn mock_creation(server: &MockServer) {
    mock_servers(server);
    server.mock(|when, then| {
        when.method(POST)
            .path("/vpc/v2/regions/fr-par/private-networks")
            .json_body_partial(r#"{"project_id":"proj-1"}"#);
        then.status(200)
            .json_body(serde_json::json!({"id": "pn-1"}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/k8s/v1/regions/fr-par/versions");
        then.status(200).json_body(serde_json::json!({
            "versions": [{"name": "1.30.2"}, {"name": "1.31.4"}, {"name": "1.9.9"}]
        }));
    });
    server.mock(|when, then| {
        when.method(POST)
            .path("/k8s/v1/regions/fr-par/clusters")
            .json_body_partial(
                r#"{"version":"1.31.4","type":"kapsule-dedicated-4","private_network_id":"pn-1","project_id":"proj-1"}"#,
            );
        then.status(200).json_body(
            serde_json::json!({"id": "cl-1", "status": "creating", "type": "kapsule-dedicated-4"}),
        );
    });
}

fn mock_pool_creation(server: &MockServer) {
    server.mock(|when, then| {
        when.method(POST)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1/pools")
            .json_body_partial(
                r#"{"node_type":"PRO2-S","size":2,"min_size":2,"max_size":4,"autoscaling":true,"zone":"fr-par-1"}"#,
            );
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1", "status": "scaling"}));
    });
}

fn mock_deletions(server: &MockServer) {
    server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/pools/pool-1");
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1", "status": "deleting"}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/k8s/v1/regions/fr-par/pools/pool-1");
        then.status(404)
            .json_body(serde_json::json!({"type": "not_found", "message": "gone"}));
    });
    server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1")
            .query_param("with_additional_resources", "true");
        then.status(200)
            .json_body(serde_json::json!({"id": "cl-1", "status": "deleting"}));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1");
        then.status(404)
            .json_body(serde_json::json!({"type": "not_found", "message": "gone"}));
    });
    server.mock(|when, then| {
        when.method(DELETE)
            .path("/vpc/v2/regions/fr-par/private-networks/pn-1");
        then.status(204);
    });
}

fn mock_ready(server: &MockServer) {
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1");
        then.status(200).json_body(
            serde_json::json!({"id": "cl-1", "status": "ready", "type": "kapsule-dedicated-4"}),
        );
    });
    server.mock(|when, then| {
        when.method(GET).path("/k8s/v1/regions/fr-par/pools/pool-1");
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1", "status": "ready"}));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1/kubeconfig")
            .query_param("dl", "1");
        then.status(200).body(KUBECONFIG);
    });
}

#[tokio::test]
async fn provision_records_every_resource_then_bootstraps() {
    let server = MockServer::start_async().await;
    mock_creation(&server);
    mock_pool_creation(&server);
    mock_ready(&server);
    let harness = harness(&server, None);

    let cluster = harness
        .provisioner
        .provision(harness.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect("provision");

    let client_id = format!("herald-{}", harness.data_plane_id);
    assert_eq!(
        cluster.herald,
        HeraldBinding {
            subject: format!("sa-{client_id}"),
            client_id,
        }
    );
    assert_eq!(cluster.capacity.cpu_millis(), 8000);
    assert_eq!(cluster.capacity.memory_mib(), 32768);
    let recorded: Vec<_> = harness.inventory.all();
    assert_eq!(
        recorded,
        vec![
            (resource(ResourceKind::PrivateNetwork, "pn-1"), false),
            (resource(ResourceKind::Cluster, "cl-1"), false),
            (resource(ResourceKind::NodePool, "pool-1"), false),
        ]
    );
    let calls = harness.bootstrapper.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, harness.data_plane_id);
    assert_eq!(calls[0].1, KUBECONFIG);
    assert_eq!(calls[0].2, 3);
}

#[tokio::test]
async fn quota_failure_is_a_readable_reason_and_rolls_back() {
    let server = MockServer::start_async().await;
    mock_servers(&server);
    server.mock(|when, then| {
        when.method(POST)
            .path("/vpc/v2/regions/fr-par/private-networks");
        then.status(200)
            .json_body(serde_json::json!({"id": "pn-1"}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/k8s/v1/regions/fr-par/versions");
        then.status(200)
            .json_body(serde_json::json!({"versions": [{"name": "1.31.4"}]}));
    });
    server.mock(|when, then| {
        when.method(POST).path("/k8s/v1/regions/fr-par/clusters");
        then.status(403).json_body(
            serde_json::json!({"type": "quotas_exceeded", "message": "quota for clusters reached"}),
        );
    });
    let delete_network = server.mock(|when, then| {
        when.method(DELETE)
            .path("/vpc/v2/regions/fr-par/private-networks/pn-1");
        then.status(204);
    });
    let harness = harness(&server, None);

    let error = harness
        .provisioner
        .provision(harness.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect_err("quota");

    assert!(matches!(
        error,
        CoreError::Provision(ProvisionError::QuotaExceeded)
    ));
    assert_eq!(
        error.to_string(),
        "the provider quota in this account does not allow this cluster"
    );
    delete_network.assert_hits_async(1).await;
    assert_eq!(
        harness.inventory.all(),
        vec![(resource(ResourceKind::PrivateNetwork, "pn-1"), true)]
    );
    assert!(harness.bootstrapper.calls().is_empty());
}

#[tokio::test]
async fn inventory_records_use_the_supplied_data_plane_id() {
    let server = MockServer::start_async().await;
    mock_creation(&server);
    mock_pool_creation(&server);
    mock_ready(&server);
    let harness = harness(&server, None);

    harness
        .provisioner
        .provision(harness.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect("provision");

    let ids = harness.inventory.data_plane_ids();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|id| *id == harness.data_plane_id));
}

static POOL_DELETED: AtomicBool = AtomicBool::new(false);
static CLUSTER_DELETED: AtomicBool = AtomicBool::new(false);

fn pool_delete_seen(request: &HttpMockRequest) -> bool {
    let hit = request.method == "DELETE" && request.path == "/k8s/v1/regions/fr-par/pools/pool-1";
    if hit {
        POOL_DELETED.store(true, Ordering::SeqCst);
    }
    hit
}

fn cluster_delete_seen(request: &HttpMockRequest) -> bool {
    let hit = request.method == "DELETE" && request.path == "/k8s/v1/regions/fr-par/clusters/cl-1";
    if hit {
        CLUSTER_DELETED.store(true, Ordering::SeqCst);
    }
    hit
}

fn pool_alive(_: &HttpMockRequest) -> bool {
    !POOL_DELETED.load(Ordering::SeqCst)
}

fn pool_dead(_: &HttpMockRequest) -> bool {
    POOL_DELETED.load(Ordering::SeqCst)
}

fn cluster_alive(_: &HttpMockRequest) -> bool {
    !CLUSTER_DELETED.load(Ordering::SeqCst)
}

fn cluster_dead(_: &HttpMockRequest) -> bool {
    CLUSTER_DELETED.load(Ordering::SeqCst)
}

#[tokio::test]
async fn pool_that_never_converges_is_reported_and_rolled_back() {
    let server = MockServer::start_async().await;
    mock_creation(&server);
    mock_pool_creation(&server);
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1")
            .matches(cluster_alive);
        then.status(200).json_body(
            serde_json::json!({"id": "cl-1", "status": "ready", "type": "kapsule-dedicated-4"}),
        );
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1")
            .matches(cluster_dead);
        then.status(404)
            .json_body(serde_json::json!({"type": "not_found", "message": "gone"}));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/pools/pool-1")
            .matches(pool_alive);
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1", "status": "scaling"}));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/pools/pool-1")
            .matches(pool_dead);
        then.status(404)
            .json_body(serde_json::json!({"type": "not_found", "message": "gone"}));
    });
    let delete_pool = server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/pools/pool-1")
            .matches(pool_delete_seen);
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1", "status": "deleting"}));
    });
    let delete_cluster = server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1")
            .matches(cluster_delete_seen);
        then.status(200)
            .json_body(serde_json::json!({"id": "cl-1", "status": "deleting"}));
    });
    let delete_network = server.mock(|when, then| {
        when.method(DELETE)
            .path("/vpc/v2/regions/fr-par/private-networks/pn-1");
        then.status(204);
    });
    let harness = harness(&server, None);

    let error = harness
        .provisioner
        .provision(harness.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect_err("never converges");

    assert!(matches!(
        error,
        CoreError::Provision(ProvisionError::NodePoolNeverConverged)
    ));
    delete_pool.assert_hits_async(1).await;
    delete_cluster.assert_hits_async(1).await;
    delete_network.assert_hits_async(1).await;
    assert!(
        harness
            .inventory
            .all()
            .iter()
            .all(|(_, released)| *released)
    );
    assert_eq!(harness.inventory.recorded(), 3);
    assert!(harness.bootstrapper.calls().is_empty());
}

#[tokio::test]
async fn failure_creating_the_second_resource_leaves_a_consistent_inventory() {
    let server = MockServer::start_async().await;
    mock_servers(&server);
    server.mock(|when, then| {
        when.method(POST)
            .path("/vpc/v2/regions/fr-par/private-networks");
        then.status(200)
            .json_body(serde_json::json!({"id": "pn-1"}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/k8s/v1/regions/fr-par/versions");
        then.status(200)
            .json_body(serde_json::json!({"versions": [{"name": "1.31.4"}]}));
    });
    let create_cluster = server.mock(|when, then| {
        when.method(POST).path("/k8s/v1/regions/fr-par/clusters");
        then.status(500)
            .json_body(serde_json::json!({"message": "internal"}));
    });
    let create_pool = server.mock(|when, then| {
        when.method(POST)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1/pools");
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1"}));
    });
    let delete_cluster = server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1");
        then.status(200)
            .json_body(serde_json::json!({"id": "cl-1"}));
    });
    let delete_network = server.mock(|when, then| {
        when.method(DELETE)
            .path("/vpc/v2/regions/fr-par/private-networks/pn-1");
        then.status(204);
    });
    let harness = harness(&server, None);

    let error = harness
        .provisioner
        .provision(harness.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect_err("cluster creation fails");

    assert!(matches!(error, CoreError::InternalError(_)));
    create_cluster.assert_hits_async(1).await;
    create_pool.assert_hits_async(0).await;
    delete_cluster.assert_hits_async(0).await;
    delete_network.assert_hits_async(1).await;
    assert_eq!(
        harness.inventory.all(),
        vec![(resource(ResourceKind::PrivateNetwork, "pn-1"), true)]
    );
}

#[tokio::test]
async fn deprovision_removes_the_recorded_resources_and_a_second_call_removes_nothing() {
    let server = MockServer::start_async().await;
    let delete_pool = server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/pools/pool-1");
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1", "status": "deleting"}));
    });
    let delete_cluster = server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1");
        then.status(200)
            .json_body(serde_json::json!({"id": "cl-1", "status": "deleting"}));
    });
    let delete_network = server.mock(|when, then| {
        when.method(DELETE)
            .path("/vpc/v2/regions/fr-par/private-networks/pn-1");
        then.status(204);
    });
    server.mock(|when, then| {
        when.method(GET);
        then.status(404)
            .json_body(serde_json::json!({"type": "not_found", "message": "gone"}));
    });
    let id = DataPlaneId(Uuid::new_v4());
    let harness = harness(
        &server,
        Some(customer_data_plane(id, CloudCredentialId(Uuid::new_v4()))),
    );
    harness
        .inventory
        .seed(id, resource(ResourceKind::PrivateNetwork, "pn-1"));
    harness
        .inventory
        .seed(id, resource(ResourceKind::Cluster, "cl-1"));
    harness
        .inventory
        .seed(id, resource(ResourceKind::NodePool, "pool-1"));

    harness.provisioner.deprovision(&id).await.expect("first");
    delete_pool.assert_hits_async(1).await;
    delete_cluster.assert_hits_async(1).await;
    delete_network.assert_hits_async(1).await;

    harness.provisioner.deprovision(&id).await.expect("second");
    delete_pool.assert_hits_async(1).await;
    delete_cluster.assert_hits_async(1).await;
    delete_network.assert_hits_async(1).await;
    assert!(
        harness
            .inventory
            .all()
            .iter()
            .all(|(_, released)| *released)
    );
}

#[tokio::test]
async fn teardown_after_a_crash_deletes_exactly_what_was_recorded() {
    let server = MockServer::start_async().await;
    mock_deletions(&server);
    let delete_pool = server.mock(|when, then| {
        when.method(DELETE)
            .path("/k8s/v1/regions/fr-par/pools/never-created");
        then.status(200).json_body(serde_json::json!({"id": "x"}));
    });
    let id = DataPlaneId(Uuid::new_v4());
    let harness = harness(
        &server,
        Some(customer_data_plane(id, CloudCredentialId(Uuid::new_v4()))),
    );
    harness
        .inventory
        .seed(id, resource(ResourceKind::PrivateNetwork, "pn-1"));
    harness
        .inventory
        .seed(id, resource(ResourceKind::Cluster, "cl-1"));

    harness
        .provisioner
        .deprovision(&id)
        .await
        .expect("teardown");

    delete_pool.assert_hits_async(0).await;
    assert_eq!(
        harness.inventory.all(),
        vec![
            (resource(ResourceKind::PrivateNetwork, "pn-1"), true),
            (resource(ResourceKind::Cluster, "cl-1"), true),
        ]
    );
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("capture lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static CAPTURE: OnceLock<Capture> = OnceLock::new();

fn capture() -> &'static Capture {
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber).ok();
        capture
    })
}

#[tokio::test]
async fn no_secret_reaches_a_log_line_or_an_error() {
    let capture = capture();

    let ok_server = MockServer::start_async().await;
    mock_creation(&ok_server);
    mock_pool_creation(&ok_server);
    mock_ready(&ok_server);
    let ok = harness(&ok_server, None);
    ok.provisioner
        .provision(ok.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect("provision");

    let failing_server = MockServer::start_async().await;
    mock_servers(&failing_server);
    failing_server.mock(|when, then| {
        when.method(POST)
            .path("/vpc/v2/regions/fr-par/private-networks");
        then.status(403)
            .json_body(serde_json::json!({"type": "denied_authentication", "message": "denied"}));
    });
    let failing = harness(&failing_server, None);
    let error = failing
        .provisioner
        .provision(failing.request(profile("kapsule-dedicated-4", "PRO2-S")))
        .await
        .expect_err("denied");
    assert!(matches!(
        error,
        CoreError::Provision(ProvisionError::CredentialRejected)
    ));

    let malformed = ScalewayVerifier::new(&config(&failing_server))
        .expect("verifier")
        .verify(
            Provider::Scaleway,
            &SecretString::new(format!("{{\"secret_key\":\"{SECRET_KEY}\"")),
        )
        .await
        .expect_err("malformed");

    let logs = String::from_utf8(capture.0.lock().expect("capture lock").clone()).expect("utf8");
    assert!(logs.contains("private network created"), "logs: {logs:?}");
    for forbidden in [SECRET_KEY, "SCWACCESSKEY", "KUBECONFIGTOKEN"] {
        assert!(!logs.contains(forbidden), "log leaks {forbidden}");
        assert!(!format!("{error} {error:?}").contains(forbidden));
        assert!(!format!("{malformed} {malformed:?}").contains(forbidden));
    }
}

fn mock_resize_state(server: &MockServer, pool_node_type: &str, cluster_type: &str) {
    server.mock(|when, then| {
        when.method(GET).path("/k8s/v1/regions/fr-par/pools/pool-1");
        then.status(200).json_body(serde_json::json!({
            "id": "pool-1", "status": "ready", "node_type": pool_node_type, "size": 3
        }));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1");
        then.status(200)
            .json_body(serde_json::json!({"id": "cl-1", "status": "ready", "type": cluster_type}));
    });
}

fn resize_harness(server: &MockServer) -> (Harness, DataPlaneId) {
    let id = DataPlaneId(Uuid::new_v4());
    let harness = harness(
        server,
        Some(customer_data_plane(id, CloudCredentialId(Uuid::new_v4()))),
    );
    harness
        .inventory
        .seed(id, resource(ResourceKind::Cluster, "cl-1"));
    harness
        .inventory
        .seed(id, resource(ResourceKind::NodePool, "pool-1"));
    (harness, id)
}

#[tokio::test]
async fn resize_updates_the_pool_bounds_and_the_control_plane() {
    let server = MockServer::start_async().await;
    mock_resize_state(&server, "PRO2-S", "kapsule");
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1/available-types");
        then.status(200)
            .json_body(serde_json::json!({"cluster_types": [{"name": "kapsule-dedicated-4"}]}));
    });
    let patch = server.mock(|when, then| {
        when.method(PATCH)
            .path("/k8s/v1/regions/fr-par/pools/pool-1")
            .json_body_partial(r#"{"autoscaling":true,"min_size":2,"max_size":4,"size":3}"#);
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1"}));
    });
    let set_type = server.mock(|when, then| {
        when.method(POST)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1/set-type")
            .json_body_partial(r#"{"type":"kapsule-dedicated-4"}"#);
        then.status(200)
            .json_body(serde_json::json!({"id": "cl-1"}));
    });
    let (harness, id) = resize_harness(&server);

    harness
        .provisioner
        .resize(&id, &profile("kapsule-dedicated-4", "PRO2-S"))
        .await
        .expect("resize");

    patch.assert_hits_async(1).await;
    set_type.assert_hits_async(1).await;
}

#[tokio::test]
async fn resize_refuses_a_node_type_change_without_touching_anything() {
    let server = MockServer::start_async().await;
    mock_resize_state(&server, "PRO2-M", "kapsule-dedicated-4");
    let patch = server.mock(|when, then| {
        when.method(PATCH);
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1"}));
    });
    let (harness, id) = resize_harness(&server);

    let error = harness
        .provisioner
        .resize(&id, &profile("kapsule-dedicated-4", "PRO2-S"))
        .await
        .expect_err("refused");

    assert!(error.to_string().contains("cannot change the node type"));
    patch.assert_hits_async(0).await;
}

#[tokio::test]
async fn resize_refuses_a_control_plane_the_cluster_cannot_reach() {
    let server = MockServer::start_async().await;
    mock_resize_state(&server, "PRO2-S", "kapsule");
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/clusters/cl-1/available-types");
        then.status(200)
            .json_body(serde_json::json!({"cluster_types": []}));
    });
    let patch = server.mock(|when, then| {
        when.method(PATCH);
        then.status(200)
            .json_body(serde_json::json!({"id": "pool-1"}));
    });
    let (harness, id) = resize_harness(&server);

    let error = harness
        .provisioner
        .resize(&id, &profile("kapsule-dedicated-4", "PRO2-S"))
        .await
        .expect_err("refused");

    assert!(error.to_string().contains("cannot move this cluster"));
    patch.assert_hits_async(0).await;
}

fn catalog(server: &MockServer) -> ScalewayCatalog<FakeStore> {
    ScalewayCatalog::new(
        config(server),
        FakeStore {
            secret: secret_json(),
        },
    )
    .expect("catalog")
}

#[tokio::test]
async fn catalog_maps_cluster_types_and_instance_prices() {
    let server = MockServer::start_async().await;
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/cluster-types")
            .header("x-auth-token", SECRET_KEY);
        then.status(200).json_body(serde_json::json!({
            "cluster_types": [
                {"name": "kapsule", "availability": "available", "dedicated": false},
                {"name": "kapsule-dedicated-4", "availability": "available", "dedicated": true},
                {"name": "kapsule-dedicated-8", "availability": "shortage", "dedicated": true},
                {"name": "kapsule-dedicated-16", "availability": "available", "dedicated": true}
            ]
        }));
    });
    mock_servers(&server);
    server.mock(|when, then| {
        when.method(GET)
            .path("/product-catalog/v2alpha1/public-catalog/products")
            .query_param("product_types", "kubernetes")
            .query_param("region", "fr-par");
        then.status(200).json_body(serde_json::json!({
            "products": [{
                "variant": "kapsule-dedicated-4",
                "price": {"retail_price": {"currency_code": "EUR", "units": 0, "nanos": 150000000}},
                "unit_of_measure": {"unit": "hour", "size": 1},
                "properties": {"kubernetes": {"kapsule_control_plane": {}}}
            }]
        }));
    });
    let catalog = catalog(&server);

    let offers = catalog
        .offers(
            Provider::Scaleway,
            &CloudCredentialId(Uuid::new_v4()),
            &Region::new("fr-par"),
        )
        .await
        .expect("offers");

    assert_eq!(
        offers.control_planes,
        vec![
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("kapsule"),
                kind: ControlPlaneKind::Mutualized,
                monthly_price: Money::ZERO,
            },
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("kapsule-dedicated-4"),
                kind: ControlPlaneKind::Dedicated,
                monthly_price: Money::new(10_800),
            },
        ]
    );
    assert_eq!(
        offers.node_types,
        vec![
            NodeOffer {
                node_type: NodeType::new("PRO2-M"),
                monthly_price: Money::new(14_400),
            },
            NodeOffer {
                node_type: NodeType::new("PRO2-S"),
                monthly_price: Money::new(7_200),
            },
        ]
    );
}

#[tokio::test]
async fn catalog_maps_a_rejected_key_and_an_unknown_region() {
    let server = MockServer::start_async().await;
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/fr-par/cluster-types");
        then.status(401)
            .json_body(serde_json::json!({"type": "denied_authentication", "message": "bad key"}));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/k8s/v1/regions/mars-1/cluster-types");
        then.status(400).json_body(
            serde_json::json!({"type": "invalid_arguments", "message": "region is invalid"}),
        );
    });
    let catalog = catalog(&server);
    let credential = CloudCredentialId(Uuid::new_v4());

    let rejected = catalog
        .offers(Provider::Scaleway, &credential, &Region::new("fr-par"))
        .await
        .expect_err("rejected");
    let unknown = catalog
        .offers(Provider::Scaleway, &credential, &Region::new("mars-1"))
        .await
        .expect_err("unknown region");

    assert!(matches!(rejected, CatalogError::CredentialRejected));
    assert!(matches!(
        unknown,
        CatalogError::RegionUnavailable { region } if region == "mars-1"
    ));
}

fn mock_iam(server: &MockServer, permission_sets: &[&str]) {
    server.mock(|when, then| {
        when.method(GET)
            .path("/iam/v1alpha1/api-keys/SCWACCESSKEY")
            .header("x-auth-token", SECRET_KEY);
        then.status(200)
            .json_body(serde_json::json!({"application_id": "app-1"}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/account/v3/projects/proj-1");
        then.status(200)
            .json_body(serde_json::json!({"organization_id": "org-1"}));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/iam/v1alpha1/policies")
            .query_param("organization_id", "org-1")
            .query_param("application_ids", "app-1");
        then.status(200)
            .json_body(serde_json::json!({"policies": [{"id": "pol-1"}]}));
    });
    let names: Vec<String> = permission_sets
        .iter()
        .map(|name| name.to_string())
        .collect();
    server.mock(|when, then| {
        when.method(GET)
            .path("/iam/v1alpha1/rules")
            .query_param("policy_id", "pol-1");
        then.status(200).json_body(serde_json::json!({
            "rules": [{"permission_set_names": names}]
        }));
    });
}

const MINIMAL: [&str; 5] = [
    "KubernetesFullAccess",
    "InstancesReadOnly",
    "PrivateNetworksFullAccess",
    "ProjectReadOnly",
    "IAMReadOnly",
];

async fn verify(server: &MockServer) -> Result<(), CredentialError> {
    ScalewayVerifier::new(&config(server))
        .expect("verifier")
        .verify(Provider::Scaleway, &SecretString::new(secret_json()))
        .await
        .map(|_| ())
}

#[tokio::test]
async fn verifier_accepts_the_minimal_permission_sets() {
    let server = MockServer::start_async().await;
    mock_iam(&server, &MINIMAL);

    verify(&server).await.expect("accepted");
}

#[tokio::test]
async fn verifier_lists_what_is_missing() {
    let server = MockServer::start_async().await;
    mock_iam(&server, &["KubernetesFullAccess", "IAMReadOnly"]);

    let error = verify(&server).await.expect_err("missing");

    assert!(matches!(
        error,
        CredentialError::MissingPermissions { missing }
            if missing == ["InstancesReadOnly", "PrivateNetworksFullAccess", "ProjectReadOnly"]
    ));
}

#[tokio::test]
async fn verifier_lists_what_is_excess() {
    let server = MockServer::start_async().await;
    let mut granted = MINIMAL.to_vec();
    granted.push("AllProductsFullAccess");
    mock_iam(&server, &granted);

    let error = verify(&server).await.expect_err("excess");

    assert!(matches!(
        error,
        CredentialError::ExcessPermissions { extra } if extra == ["AllProductsFullAccess"]
    ));
}

#[tokio::test]
async fn verifier_rejects_an_unauthenticated_key_and_a_malformed_payload() {
    let server = MockServer::start_async().await;
    server.mock(|when, then| {
        when.method(GET).path("/iam/v1alpha1/api-keys/SCWACCESSKEY");
        then.status(401)
            .json_body(serde_json::json!({"type": "denied_authentication", "message": "bad key"}));
    });

    assert!(matches!(
        verify(&server).await,
        Err(CredentialError::Invalid)
    ));
    let malformed = ScalewayVerifier::new(&config(&server))
        .expect("verifier")
        .verify(Provider::Scaleway, &SecretString::new("not json"))
        .await;
    assert!(matches!(malformed, Err(CredentialError::Invalid)));
}

#[tokio::test]
async fn verifier_asks_for_iam_read_when_the_key_cannot_be_inspected() {
    let server = MockServer::start_async().await;
    server.mock(|when, then| {
        when.method(GET).path("/iam/v1alpha1/api-keys/SCWACCESSKEY");
        then.status(403).json_body(
            serde_json::json!({"type": "permissions_denied", "message": "no iam access"}),
        );
    });

    assert!(matches!(
        verify(&server).await,
        Err(CredentialError::MissingPermissions { missing }) if missing == ["IAMReadOnly"]
    ));
}
