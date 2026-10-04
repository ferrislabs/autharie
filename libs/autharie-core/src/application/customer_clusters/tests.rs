use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::Duration,
};

use autharie_domain::{
    CoreError,
    dataplane::{
        cloud_provider::{
            ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeType,
        },
        cluster_profile::{ClusterMode, ClusterProfile, Replication},
        credential::{CloudCredentialId, CredentialError},
        entities::DataPlane,
        herald_identity::{HeraldBinding, MintedHeraldIdentity},
        ports::HeraldIdentityProvisioner,
        provisioner::{
            ClusterProvisioner, ProvisionError, ProvisionRequest, ProvisionTarget,
            ProvisionedCluster,
        },
        value_objects::{
            Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneStatus, DeploymentResources,
            Region,
        },
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use super::{
    ClaimedCluster, CustomerClusterQueue, CustomerClusterWorker, ProvisionReport, TeardownReport,
    WorkerSettings,
};

const SECRET: &str = "SCW-SECRET-KEY-0123456789";

type Journal = Arc<Mutex<Vec<String>>>;

fn profile() -> ClusterProfile {
    ClusterProfile::restore(
        ClusterMode::Standard,
        ControlPlaneOffer {
            id: ControlPlaneOfferId::new("kapsule"),
            kind: ControlPlaneKind::Mutualized,
            monthly_price: Money::new(0),
        },
        NodeType::new("PRO2-S"),
        2,
        4,
        Replication::new(2).expect("replicas"),
    )
    .expect("a valid profile")
}

fn customer_plane() -> DataPlane {
    DataPlane::new(
        DataPlaneAllocation::Customer {
            organisation_id: OrganisationId(Uuid::new_v4()),
            deployment_id: DeploymentId(Uuid::new_v4()),
            credential_id: CloudCredentialId(Uuid::new_v4()),
        },
        Region::new("fr-par"),
        Capacity::new(1, 1, 1).expect("capacity"),
    )
}

fn claimed(plane: DataPlane) -> ClaimedCluster {
    let DataPlaneAllocation::Customer {
        deployment_id,
        credential_id,
        ..
    } = plane.allocation
    else {
        unreachable!("a customer plane")
    };
    ClaimedCluster {
        data_plane: plane,
        deployment_id,
        credential_id,
        profile: profile(),
        resources: DeploymentResources::DEFAULT,
    }
}

fn built() -> ProvisionedCluster {
    ProvisionedCluster {
        herald: HeraldBinding {
            client_id: "herald-x".to_string(),
            subject: "sub-x".to_string(),
        },
        capacity: Capacity::new(8_000, 32_768, 20).expect("capacity"),
    }
}

#[derive(Default)]
struct QueueState {
    claims: Vec<ClaimedCluster>,
    planes: Vec<DataPlane>,
    completed: Vec<DataPlane>,
    failed: Vec<(DataPlaneId, String)>,
    disabled: Vec<DataPlaneId>,
    deleting: Vec<DataPlaneId>,
    confirmed: Vec<DataPlaneId>,
    confirmations_refused: u32,
    complete_applies: bool,
    leases: Vec<Duration>,
}

#[derive(Clone)]
struct FakeQueue {
    state: Arc<Mutex<QueueState>>,
}

impl FakeQueue {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(QueueState {
                complete_applies: true,
                ..QueueState::default()
            })),
        }
    }

    fn with_claim(self, cluster: ClaimedCluster) -> Self {
        self.state.lock().expect("lock").claims.push(cluster);
        self
    }

    fn with_plane(self, plane: DataPlane) -> Self {
        self.state.lock().expect("lock").planes.push(plane);
        self
    }

    fn with_deleting_deployment(self, id: DataPlaneId) -> Self {
        self.state.lock().expect("lock").deleting.push(id);
        self
    }

    fn refusing_confirmations(self, times: u32) -> Self {
        self.state.lock().expect("lock").confirmations_refused = times;
        self
    }

    fn confirmed(&self) -> Vec<DataPlaneId> {
        self.state.lock().expect("lock").confirmed.clone()
    }

    fn refusing_completion(self) -> Self {
        self.state.lock().expect("lock").complete_applies = false;
        self
    }

    fn completed(&self) -> Vec<DataPlane> {
        self.state.lock().expect("lock").completed.clone()
    }

    fn failed(&self) -> Vec<(DataPlaneId, String)> {
        self.state.lock().expect("lock").failed.clone()
    }

    fn disabled(&self) -> Vec<DataPlaneId> {
        self.state.lock().expect("lock").disabled.clone()
    }

    fn leases(&self) -> Vec<Duration> {
        self.state.lock().expect("lock").leases.clone()
    }
}

impl CustomerClusterQueue for FakeQueue {
    async fn claim(&self, lease: Duration, _limit: u32) -> Result<Vec<ClaimedCluster>, CoreError> {
        let mut state = self.state.lock().expect("lock");
        state.leases.push(lease);
        Ok(std::mem::take(&mut state.claims))
    }

    async fn complete(&self, data_plane: &DataPlane) -> Result<bool, CoreError> {
        let mut state = self.state.lock().expect("lock");
        if state.complete_applies {
            state.completed.push(data_plane.clone());
        }
        Ok(state.complete_applies)
    }

    async fn fail(&self, id: &DataPlaneId, reason: &str) -> Result<bool, CoreError> {
        self.state
            .lock()
            .expect("lock")
            .failed
            .push((*id, reason.to_string()));
        Ok(true)
    }

    async fn teardown_candidates(&self, _limit: u32) -> Result<Vec<DataPlane>, CoreError> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .planes
            .iter()
            .filter(|plane| plane.status != DataPlaneStatus::Disabled)
            .cloned()
            .collect())
    }

    async fn disable(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        let mut state = self.state.lock().expect("lock");
        state.disabled.push(*id);
        for plane in state.planes.iter_mut().filter(|plane| plane.id == *id) {
            plane.disable();
        }
        Ok(true)
    }

    async fn awaiting_deletion(&self, _limit: u32) -> Result<Vec<DataPlaneId>, CoreError> {
        Ok(self.deletion_pending())
    }

    async fn confirm_deleted(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        let mut state = self.state.lock().expect("lock");
        if state.confirmations_refused > 0 {
            state.confirmations_refused -= 1;
            return Err(CoreError::InternalError("database down".to_string()));
        }
        let Some(position) = state.deleting.iter().position(|pending| pending == id) else {
            return Ok(false);
        };
        state.deleting.remove(position);
        state.confirmed.push(*id);
        Ok(true)
    }
}

impl FakeQueue {
    fn deletion_pending(&self) -> Vec<DataPlaneId> {
        let state = self.state.lock().expect("lock");
        state
            .planes
            .iter()
            .filter(|plane| {
                matches!(
                    plane.status,
                    DataPlaneStatus::Disabled | DataPlaneStatus::Failed
                ) && state.deleting.contains(&plane.id)
            })
            .map(|plane| plane.id)
            .collect()
    }
}

struct FakeProvisioner {
    journal: Journal,
    provision: Mutex<Vec<Result<ProvisionedCluster, CoreError>>>,
    deprovision: Mutex<Vec<Result<(), CoreError>>>,
    requests: Mutex<Vec<(DataPlaneId, OrganisationId, Region, DeploymentResources)>>,
    targets: Mutex<Vec<(CloudCredentialId, DeploymentId)>>,
}

impl FakeProvisioner {
    fn new(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            provision: Mutex::new(Vec::new()),
            deprovision: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            targets: Mutex::new(Vec::new()),
        }
    }

    fn provisioning(self, outcome: Result<ProvisionedCluster, CoreError>) -> Self {
        self.provision.lock().expect("lock").push(outcome);
        self
    }

    fn deprovisioning(self, outcome: Result<(), CoreError>) -> Self {
        self.deprovision.lock().expect("lock").push(outcome);
        self
    }
}

impl ClusterProvisioner for &FakeProvisioner {
    async fn provision(&self, request: ProvisionRequest) -> Result<ProvisionedCluster, CoreError> {
        self.journal.lock().expect("lock").push("provision".into());
        self.requests.lock().expect("lock").push((
            request.data_plane_id,
            request.organisation_id,
            request.region.clone(),
            request.minimum,
        ));
        if let ProvisionTarget::Customer {
            credential_id,
            deployment_id,
            ..
        } = request.target
        {
            self.targets
                .lock()
                .expect("lock")
                .push((credential_id, deployment_id));
        }
        let mut outcomes = self.provision.lock().expect("lock");
        if outcomes.is_empty() {
            return Ok(built());
        }
        outcomes.remove(0)
    }

    async fn deprovision(&self, _id: &DataPlaneId) -> Result<(), CoreError> {
        self.journal
            .lock()
            .expect("lock")
            .push("deprovision".into());
        let mut outcomes = self.deprovision.lock().expect("lock");
        if outcomes.is_empty() {
            return Ok(());
        }
        outcomes.remove(0)
    }

    async fn resize(&self, _id: &DataPlaneId, _profile: &ClusterProfile) -> Result<(), CoreError> {
        Ok(())
    }
}

struct FakeIdentities {
    journal: Journal,
    refusing: Mutex<u32>,
}

impl FakeIdentities {
    fn new(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            refusing: Mutex::new(0),
        }
    }
}

impl HeraldIdentityProvisioner for &FakeIdentities {
    async fn mint(&self, _dataplane: DataPlaneId) -> Result<MintedHeraldIdentity, CoreError> {
        Err(CoreError::InternalError("not minted here".to_string()))
    }

    async fn revoke(&self, _dataplane: DataPlaneId) -> Result<(), CoreError> {
        self.journal.lock().expect("lock").push("revoke".into());
        let mut refusing = self.refusing.lock().expect("lock");
        if *refusing > 0 {
            *refusing -= 1;
            return Err(CoreError::InternalError("realm unreachable".to_string()));
        }
        Ok(())
    }
}

fn settings() -> WorkerSettings {
    WorkerSettings {
        claim_lease: Duration::from_secs(900),
        batch: 4,
    }
}

fn journal() -> Journal {
    Journal::default()
}

fn steps(journal: &Journal) -> Vec<String> {
    journal.lock().expect("lock").clone()
}

#[tokio::test]
async fn a_provisioned_cluster_gets_its_binding_and_capacity_and_stays_provisioning() {
    let journal = journal();
    let plane = customer_plane();
    let id = plane.id;
    let queue = FakeQueue::new().with_claim(claimed(plane));
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let report = worker.provision_pending().await.expect("a pass");

    assert_eq!(
        report,
        ProvisionReport {
            provisioned: 1,
            failed: 0,
            deferred: 0
        }
    );
    let completed = queue.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].id, id);
    assert_eq!(completed[0].status, DataPlaneStatus::Provisioning);
    assert_eq!(completed[0].herald, Some(built().herald));
    assert_eq!(completed[0].capacity, built().capacity);
    assert!(queue.failed().is_empty());
    assert_eq!(queue.leases(), vec![Duration::from_secs(900)]);
}

#[tokio::test]
async fn the_request_is_built_from_the_plane_and_the_deployment() {
    let journal = journal();
    let plane = customer_plane();
    let cluster = claimed(plane.clone());
    let queue = FakeQueue::new().with_claim(cluster.clone());
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue, &provisioner, &identities, settings());

    worker.provision_pending().await.expect("a pass");

    let requests = provisioner.requests.lock().expect("lock");
    assert_eq!(
        requests[0],
        (
            plane.id,
            plane.allocation.owner().expect("owner"),
            Region::new("fr-par"),
            DeploymentResources::DEFAULT
        )
    );
    assert_eq!(
        provisioner.targets.lock().expect("lock")[0],
        (cluster.credential_id, cluster.deployment_id)
    );
}

#[tokio::test]
async fn leftovers_of_an_earlier_attempt_are_released_before_building_again() {
    let journal = journal();
    let queue = FakeQueue::new().with_claim(claimed(customer_plane()));
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue, &provisioner, &identities, settings());

    worker.provision_pending().await.expect("a pass");

    assert_eq!(steps(&journal), vec!["deprovision", "provision"]);
}

#[tokio::test]
async fn leftovers_that_cannot_be_released_defer_the_plane_without_failing_it() {
    let journal = journal();
    let queue = FakeQueue::new().with_claim(claimed(customer_plane()));
    let provisioner = FakeProvisioner::new(&journal)
        .deprovisioning(Err(CoreError::InternalError("api down".to_string())));
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let report = worker.provision_pending().await.expect("a pass");

    assert_eq!(report.deferred, 1);
    assert_eq!(steps(&journal), vec!["deprovision"]);
    assert!(queue.failed().is_empty());
    assert!(queue.completed().is_empty());
}

#[tokio::test]
async fn each_provision_error_ends_as_a_failed_plane_with_its_readable_reason_spec_ccp_11() {
    let cases = [
        ProvisionError::QuotaExceeded,
        ProvisionError::RegionUnavailable,
        ProvisionError::NodePoolNeverConverged,
        ProvisionError::CredentialRejected,
    ];

    for error in cases {
        let journal = journal();
        let plane = customer_plane();
        let id = plane.id;
        let queue = FakeQueue::new().with_claim(claimed(plane));
        let provisioner = FakeProvisioner::new(&journal).provisioning(Err(error.clone().into()));
        let identities = FakeIdentities::new(&journal);
        let worker =
            CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

        let report = worker.provision_pending().await.expect("a pass");

        assert_eq!(report.failed, 1, "{error}");
        assert_eq!(queue.failed(), vec![(id, error.to_string())]);
        assert!(queue.completed().is_empty());
    }
}

#[tokio::test]
async fn a_credential_the_store_cannot_find_is_said_plainly() {
    let journal = journal();
    let plane = customer_plane();
    let credential = CloudCredentialId(Uuid::new_v4());
    let queue = FakeQueue::new().with_claim(claimed(plane));
    let provisioner = FakeProvisioner::new(&journal)
        .provisioning(Err(CredentialError::NotFound { id: credential }.into()));
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    worker.provision_pending().await.expect("a pass");

    assert_eq!(
        queue.failed()[0].1,
        format!("credential {credential} does not exist")
    );
}

#[tokio::test]
async fn an_untyped_failure_never_reaches_the_reason_verbatim() {
    let journal = journal();
    let queue = FakeQueue::new().with_claim(claimed(customer_plane()));
    let provisioner = FakeProvisioner::new(&journal)
        .provisioning(Err(CoreError::InternalError(format!("helm said {SECRET}"))));
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    worker.provision_pending().await.expect("a pass");

    let reason = &queue.failed()[0].1;
    assert!(!reason.contains(SECRET));
    assert_eq!(
        reason,
        "the cluster could not be created because of an internal error"
    );
}

#[tokio::test]
async fn a_plane_failed_or_disabled_meanwhile_is_left_as_it_is() {
    let journal = journal();
    let queue = FakeQueue::new()
        .with_claim(claimed(customer_plane()))
        .refusing_completion();
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let report = worker.provision_pending().await.expect("a pass");

    assert_eq!(report.deferred, 1);
    assert!(queue.completed().is_empty());
    assert!(queue.failed().is_empty());
}

fn deleted_plane_with_herald() -> DataPlane {
    let mut plane = customer_plane();
    plane.herald = Some(built().herald);
    plane
}

#[tokio::test]
async fn teardown_releases_the_cluster_revokes_the_identity_and_disables_the_plane_spec_ccp_13() {
    let journal = journal();
    let plane = deleted_plane_with_herald();
    let id = plane.id;
    let queue = FakeQueue::new().with_plane(plane);
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let first = worker.teardown_released().await.expect("a pass");

    assert_eq!(
        first,
        TeardownReport {
            released: 1,
            retrying: 0,
            confirmed: 0
        }
    );
    assert_eq!(steps(&journal), vec!["revoke", "deprovision"]);
    assert_eq!(queue.disabled(), vec![id]);
}

#[tokio::test]
async fn a_second_teardown_pass_does_nothing_spec_ccp_14() {
    let journal = journal();
    let queue = FakeQueue::new().with_plane(deleted_plane_with_herald());
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());
    worker.teardown_released().await.expect("a pass");
    let before = steps(&journal);

    let second = worker.teardown_released().await.expect("a pass");

    assert_eq!(second, TeardownReport::default());
    assert_eq!(steps(&journal), before);
    assert_eq!(queue.disabled().len(), 1);
}

#[tokio::test]
async fn a_plane_without_a_recorded_binding_is_still_revoked_for_an_orphaned_identity() {
    let journal = journal();
    let queue = FakeQueue::new().with_plane(customer_plane());
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue, &provisioner, &identities, settings());

    worker.teardown_released().await.expect("a pass");

    assert_eq!(steps(&journal), vec!["revoke", "deprovision"]);
}

#[tokio::test]
async fn a_failing_teardown_keeps_the_plane_and_is_retried() {
    let journal = journal();
    let plane = deleted_plane_with_herald();
    let id = plane.id;
    let queue = FakeQueue::new().with_plane(plane);
    let provisioner = FakeProvisioner::new(&journal)
        .deprovisioning(Err(CoreError::InternalError("api down".to_string())));
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let first = worker.teardown_released().await.expect("a pass");

    assert_eq!(
        first,
        TeardownReport {
            released: 0,
            retrying: 1,
            confirmed: 0
        }
    );
    assert!(queue.disabled().is_empty());

    let second = worker.teardown_released().await.expect("a pass");

    assert_eq!(
        second,
        TeardownReport {
            released: 1,
            retrying: 0,
            confirmed: 0
        }
    );
    assert_eq!(queue.disabled(), vec![id]);
}

#[tokio::test]
async fn a_revocation_that_fails_stops_the_teardown_before_the_plane_is_disabled() {
    let journal = journal();
    let queue = FakeQueue::new().with_plane(deleted_plane_with_herald());
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    *identities.refusing.lock().expect("lock") = 1;
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let report = worker.teardown_released().await.expect("a pass");

    assert_eq!(report.retrying, 1);
    assert!(queue.disabled().is_empty());
}

#[tokio::test]
async fn a_failed_plane_is_released_but_keeps_its_failed_status_and_reason() {
    let journal = journal();
    let mut plane = customer_plane();
    plane.fail("the provider quota in this account does not allow this cluster");
    let queue = FakeQueue::new().with_plane(plane);
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let report = worker.teardown_released().await.expect("a pass");

    assert_eq!(report.released, 1);
    assert_eq!(steps(&journal), vec!["revoke", "deprovision"]);
    assert!(queue.disabled().is_empty());
}

#[tokio::test]
async fn a_released_cluster_marks_its_deployment_deleted_and_a_second_pass_does_nothing() {
    let journal = journal();
    let plane = deleted_plane_with_herald();
    let id = plane.id;
    let queue = FakeQueue::new()
        .with_plane(plane)
        .with_deleting_deployment(id);
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let first = worker.teardown_released().await.expect("a pass");
    let second = worker.teardown_released().await.expect("a pass");

    assert_eq!(
        first,
        TeardownReport {
            released: 1,
            retrying: 0,
            confirmed: 1
        }
    );
    assert_eq!(second, TeardownReport::default());
    assert_eq!(queue.confirmed(), vec![id]);
}

#[tokio::test]
async fn a_failed_plane_whose_deployment_was_deleted_ends_deleted_too() {
    let journal = journal();
    let mut plane = customer_plane();
    plane.fail("the provider quota in this account does not allow this cluster");
    let id = plane.id;
    let queue = FakeQueue::new()
        .with_plane(plane)
        .with_deleting_deployment(id);
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    worker.teardown_released().await.expect("a pass");

    assert_eq!(queue.confirmed(), vec![id]);
}

#[tokio::test]
async fn a_failed_status_update_is_retried_alone_after_the_teardown_stays_done() {
    let journal = journal();
    let plane = deleted_plane_with_herald();
    let id = plane.id;
    let queue = FakeQueue::new()
        .with_plane(plane)
        .with_deleting_deployment(id)
        .refusing_confirmations(1);
    let provisioner = FakeProvisioner::new(&journal);
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue.clone(), &provisioner, &identities, settings());

    let first = worker.teardown_released().await.expect("a pass");
    let after_first = steps(&journal);

    assert_eq!(first.released, 1);
    assert_eq!(first.retrying, 1);
    assert!(queue.confirmed().is_empty());
    assert_eq!(queue.disabled(), vec![id]);

    let second = worker.teardown_released().await.expect("a pass");

    assert_eq!(
        second,
        TeardownReport {
            released: 0,
            retrying: 0,
            confirmed: 1
        }
    );
    assert_eq!(steps(&journal), after_first);
    assert_eq!(queue.confirmed(), vec![id]);
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

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

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn the_worker_logs_what_happened_and_never_a_secret() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let journal = journal();
    let queue = FakeQueue::new()
        .with_claim(claimed(customer_plane()))
        .with_plane(deleted_plane_with_herald());
    let provisioner = FakeProvisioner::new(&journal)
        .provisioning(Err(ProvisionError::CredentialRejected.into()))
        .deprovisioning(Ok(()))
        .deprovisioning(Err(CoreError::InternalError("api down".to_string())));
    let identities = FakeIdentities::new(&journal);
    let worker = CustomerClusterWorker::new(queue, &provisioner, &identities, settings());

    worker.provision_pending().await.expect("a pass");
    worker.teardown_released().await.expect("a pass");

    let logs = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("utf8 logs");
    assert!(logs.contains("provisioning a customer cluster failed"));
    assert!(logs.contains("the provider rejected the credential"));
    assert!(logs.contains("tearing a customer cluster down failed"));
    assert!(!logs.contains(SECRET));
}
