use std::fmt;

use autharie_core::{
    FixedVerdict,
    cloud_credentials::{
        DeleteCloudCredential, EstimateClusterCost, ListCloudCredentials, ProfileSpec,
        RegisterCloudCredential, ResizeCustomerCluster, ResolveCustomerCloud,
    },
    customer_clusters::{ProvisionReport, TeardownReport},
};
use autharie_domain::{
    CoreError,
    dataplane::{
        cloud_provider::{ControlPlaneOfferId, NodeType, Provider},
        cluster_profile::{
            ClusterMode, ClusterProfile, CostEstimate, ProfileError, Replication, ResizeError,
        },
        credential::{CloudCredential, CloudCredentialStore, CredentialError, SecretString},
        entities::DataPlane,
        herald_identity::HeraldBinding,
        ports::DataPlaneRepository,
        provisioner::ClusterProvisioner,
        value_objects::{Capacity, DataPlaneStatus, PlacementWindows, Region},
    },
    deployments::{
        Deployment, DeploymentKind, DeploymentName,
        commands::CreateDeploymentCommand,
        distribution::{Distribution, DistributionError},
        environment::Environment,
        ports::DeploymentService,
        service::DeploymentServiceImpl,
    },
    offers::Offer,
    organisation::{OrganisationId, value_objects::Plan},
    user::UserId,
    version::Version,
};
use autharie_e2e::support::{
    customer_cloud::{
        AllowAll, Audit, Grants, Identities, Keys, MEDIUM, MUTUALIZED, NeverProvisions, Platform,
        REGION, Runner, SMALL, Users, capture, credential_json, member, providers, provisioner,
        worker,
    },
    iam::OneOrganisation,
    platform::caller,
    scaleway_api::{
        self, CreationHits, PRO2_S_CPUS, PRO2_S_MEMORY_MIB, RefusalHits, TeardownHits, kubeconfig,
    },
};
use chrono::Utc;
use cucumber::{World, given, then, when};
use httpmock::MockServer;
use serde_json::json;
use uuid::Uuid;

#[derive(Default)]
struct Server(Option<MockServer>);

impl Server {
    async fn get(&mut self) -> &MockServer {
        if self.0.is_none() {
            self.0 = Some(MockServer::start_async().await);
        }
        self.0.as_ref().expect("a started server")
    }
}

impl fmt::Debug for Server {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Server")
    }
}

type Service =
    DeploymentServiceImpl<Platform, Users, Platform, OneOrganisation, NeverProvisions, AllowAll>;

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct CustomerCloudWorld {
    platform: Platform,
    keys: Keys,
    identities: Identities,
    runner: Runner,
    spy: NeverProvisions,
    server: Server,
    organisation: OrganisationId,
    verdict: FixedVerdict,
    secret_key: String,
    kubeconfig_token: String,
    registrations: Vec<Result<CloudCredential, CoreError>>,
    listed: Option<Vec<CloudCredential>>,
    credential: Option<CloudCredential>,
    credential_deletion: Option<Result<(), CoreError>>,
    distribution: Option<Result<Distribution, CoreError>>,
    estimate: Option<Result<CostEstimate, CoreError>>,
    profile: Option<ClusterProfile>,
    request: Option<Result<CreateDeploymentCommand, DistributionError>>,
    deployment: Option<Deployment>,
    provision_reports: Vec<ProvisionReport>,
    creation_hits: Vec<CreationHits>,
    refusal_hits: Option<RefusalHits>,
    teardown_reports: Vec<TeardownReport>,
    teardown_hits: Vec<TeardownHits>,
    direct_deprovisions: Vec<Result<(), CoreError>>,
    direct_hits: Option<TeardownHits>,
    plane_before_resize: Option<DataPlane>,
    patches: usize,
    resize_outcome: Option<Result<ClusterProfile, CoreError>>,
    audit: Audit,
}

impl CustomerCloudWorld {
    fn new() -> Self {
        capture();
        Self {
            platform: Platform::default(),
            keys: Keys::default(),
            identities: Identities::default(),
            runner: Runner::default(),
            spy: NeverProvisions::default(),
            server: Server::default(),
            organisation: OrganisationId(Uuid::new_v4()),
            verdict: FixedVerdict::Accept,
            secret_key: format!("SCWSECRET-{}", Uuid::new_v4().simple()),
            kubeconfig_token: format!("KUBETOKEN-{}", Uuid::new_v4().simple()),
            registrations: Vec::new(),
            listed: None,
            credential: None,
            credential_deletion: None,
            distribution: None,
            estimate: None,
            profile: None,
            request: None,
            deployment: None,
            provision_reports: Vec::new(),
            creation_hits: Vec::new(),
            refusal_hits: None,
            teardown_reports: Vec::new(),
            teardown_hits: Vec::new(),
            direct_deprovisions: Vec::new(),
            direct_hits: None,
            plane_before_resize: None,
            patches: 0,
            resize_outcome: None,
            audit: Audit::default(),
        }
    }

    fn store(&self) -> autharie_core::EnvelopeCredentialStore<'_, Platform, Keys> {
        autharie_core::EnvelopeCredentialStore::new(self.platform.clone(), &self.keys)
            .expect("a credential store")
    }

    fn service(&self) -> Service {
        DeploymentServiceImpl::new(
            self.platform.clone(),
            Users,
            self.platform.clone(),
            OneOrganisation::on(Plan::Free),
            self.spy.clone(),
            PlacementWindows::new(chrono::Duration::seconds(90), chrono::Duration::minutes(30)),
            AllowAll,
        )
    }

    fn secret(&self) -> String {
        credential_json(&self.secret_key)
    }

    async fn register(&mut self, label: &str) -> Result<CloudCredential, CoreError> {
        let verifier = providers(self.verdict.clone());
        let secret = SecretString::new(self.secret());
        RegisterCloudCredential::new(
            self.store(),
            &verifier,
            self.platform.clone(),
            Grants::administrator(),
        )
        .execute(
            caller(),
            self.organisation,
            Provider::Scaleway,
            label.to_string(),
            secret,
        )
        .await
    }

    async fn resolve(&self, spec: ProfileSpec) -> Result<Distribution, CoreError> {
        let catalog = providers(FixedVerdict::Accept);
        let credential = self.credential.as_ref().expect("a registered credential");
        ResolveCustomerCloud::new(&catalog, self.platform.clone(), Grants::administrator())
            .execute(
                caller(),
                self.organisation,
                credential.id,
                &Region::new(REGION),
                spec,
            )
            .await
    }

    async fn create_on_customer_cloud(&mut self, mode: ClusterMode) {
        let distribution = self
            .resolve(spec_for(mode))
            .await
            .expect("a valid distribution");
        if let Distribution::CustomerCloud { profile, .. } = &distribution {
            self.profile = Some(profile.clone());
        }
        let command = CreateDeploymentCommand::new(
            self.organisation,
            DeploymentName("tenant".to_string()),
            DeploymentKind::Ferriskey,
            Version::new(26, 0, 1),
            UserId(Uuid::from_u128(9)),
            Environment::Development,
            Region::new(REGION),
            Offer::Sandbox,
        )
        .expect("a creatable name")
        .with_distribution(distribution)
        .expect("ferriskey may use the customer cloud");
        self.deployment = Some(
            self.service()
                .create_deployment(caller(), command)
                .await
                .expect("the deployment is recorded"),
        );
    }

    fn deployment(&self) -> &Deployment {
        self.deployment.as_ref().expect("a deployment")
    }

    fn plane(&self) -> DataPlane {
        self.platform
            .plane(self.deployment().dataplane_id)
            .expect("the data plane of the deployment")
    }

    fn deployed_profile(&self) -> ClusterProfile {
        match &self.deployment().distribution {
            Distribution::CustomerCloud { profile, .. } => profile.clone(),
            other => panic!("not a customer cloud deployment: {other:?}"),
        }
    }

    async fn provision(&mut self) {
        let profile = self.deployed_profile();
        let kubeconfig = kubeconfig(&self.kubeconfig_token);
        let secret_key = self.secret_key.clone();
        let server = self.server.get().await;
        let creation = scaleway_api::creation(server, &secret_key, &profile, &kubeconfig).await;
        let report = worker(
            &server.base_url(),
            &self.platform,
            &self.keys,
            &self.identities,
            &self.runner,
        )
        .provision_pending()
        .await
        .expect("a provisioning pass");
        let hits = creation.hits().await;
        creation.remove().await;
        self.provision_reports.push(report);
        self.creation_hits.push(hits);
    }

    async fn heartbeat(&self) {
        let seen = self
            .platform
            .touch_last_seen(&self.deployment().dataplane_id, Utc::now(), None, None)
            .await
            .expect("a heartbeat");
        assert!(seen, "the heartbeat found no data plane");
    }

    async fn tear_down(&mut self) {
        let server = self.server.get().await;
        let teardown = scaleway_api::teardown(server).await;
        let report = worker(
            &server.base_url(),
            &self.platform,
            &self.keys,
            &self.identities,
            &self.runner,
        )
        .teardown_released()
        .await
        .expect("a teardown pass");
        let hits = teardown.hits().await;
        teardown.remove().await;
        self.teardown_reports.push(report);
        self.teardown_hits.push(hits);
    }

    async fn provisioning_status(&self) -> serde_json::Value {
        let view = self
            .service()
            .get_provisioning_for_organisation(caller(), self.organisation, self.deployment().id)
            .await
            .expect("the provisioning view")
            .expect("a customer cloud deployment");
        serde_json::to_value(view).expect("json")
    }

    fn nothing_created(&self) {
        assert!(self.platform.deployments().is_empty());
        assert!(self.platform.planes().is_empty());
        assert!(self.platform.inventory().is_empty());
        assert_eq!(self.spy.calls(), 0);
    }
}

fn spec_for(mode: ClusterMode) -> ProfileSpec {
    let (min, max, replicas) = match mode {
        ClusterMode::Dev => (1, 1, 1),
        ClusterMode::Standard => (2, 4, 2),
        ClusterMode::Ha => (3, 10, 2),
    };
    spec(mode, MUTUALIZED, SMALL, min, max, replicas)
}

fn spec(
    mode: ClusterMode,
    control_plane: &str,
    node_type: &str,
    min: u8,
    max: u8,
    replicas: u8,
) -> ProfileSpec {
    ProfileSpec {
        mode,
        control_plane_id: ControlPlaneOfferId::new(control_plane),
        node_type: NodeType::new(node_type),
        min_nodes: min,
        max_nodes: max,
        replication: Replication::new(replicas).expect("at least one replica"),
    }
}

fn mode_named(name: &str) -> ClusterMode {
    match name {
        "dev" => ClusterMode::Dev,
        "standard" => ClusterMode::Standard,
        "ha" => ClusterMode::Ha,
        other => panic!("no cluster mode named {other}"),
    }
}

fn kind_named(name: &str) -> DeploymentKind {
    match name {
        "ferriskey" => DeploymentKind::Ferriskey,
        "keycloak" => DeploymentKind::Keycloak,
        other => panic!("no deployment kind named {other}"),
    }
}

fn variant(error: &ProfileError) -> &'static str {
    match error {
        ProfileError::NodeRangeInvalid { .. } => "NodeRangeInvalid",
        ProfileError::BelowModeFloor { .. } => "BelowModeFloor",
        ProfileError::AboveModeCeiling { .. } => "AboveModeCeiling",
        ProfileError::ReplicationExceedsNodes { .. } => "ReplicationExceedsNodes",
        ProfileError::NodeTypeUnavailable { .. } => "NodeTypeUnavailable",
        ProfileError::ControlPlaneUnavailable { .. } => "ControlPlaneUnavailable",
        ProfileError::ControlPlaneNotAllowedForMode { .. } => "ControlPlaneNotAllowedForMode",
    }
}

fn profile_refusal(outcome: &Result<Distribution, CoreError>) -> &ProfileError {
    match outcome {
        Err(CoreError::Profile(error)) => error,
        other => panic!("expected a refused profile, got {other:?}"),
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[given("a provider whose verification accepts the credential")]
fn a_provider_that_accepts(world: &mut CustomerCloudWorld) {
    world.verdict = FixedVerdict::Accept;
}

#[given(expr = "a provider reporting the missing permission {string}")]
fn a_provider_reporting_missing(world: &mut CustomerCloudWorld, permission: String) {
    world.verdict = FixedVerdict::Missing(vec![permission]);
}

#[given("a registered credential")]
async fn a_registered_credential(world: &mut CustomerCloudWorld) {
    let credential = world.register("production").await.expect("registered");
    world.credential = Some(credential);
}

#[given("a customer cloud deployment on that credential")]
async fn a_customer_cloud_deployment(world: &mut CustomerCloudWorld) {
    world.create_on_customer_cloud(ClusterMode::Standard).await;
}

#[given("a customer cloud deployment whose cluster is provisioned and active")]
async fn a_provisioned_deployment(world: &mut CustomerCloudWorld) {
    world.create_on_customer_cloud(ClusterMode::Standard).await;
    world.provision().await;
    world.heartbeat().await;
}

#[given(
    expr = "a customer cloud deployment on a {string} cluster whose cluster is provisioned and active"
)]
async fn a_provisioned_deployment_in_mode(world: &mut CustomerCloudWorld, mode: String) {
    world.create_on_customer_cloud(mode_named(&mode)).await;
    world.provision().await;
    world.heartbeat().await;
}

#[when(expr = "the customer registers the credential labelled {string}")]
async fn the_customer_registers(world: &mut CustomerCloudWorld, label: String) {
    let outcome = world.register(&label).await;
    if let Ok(credential) = &outcome {
        world.credential = Some(credential.clone());
    }
    world.registrations.push(outcome);
}

#[when(
    expr = "the provider now reports the excess permission {string} and the customer registers again"
)]
async fn the_provider_now_reports_excess(world: &mut CustomerCloudWorld, permission: String) {
    world.verdict = FixedVerdict::Excess(vec![permission]);
    let outcome = world.register("production").await;
    world.registrations.push(outcome);
}

#[when("the provider now rejects the credential as invalid and the customer registers again")]
async fn the_provider_now_rejects(world: &mut CustomerCloudWorld) {
    world.verdict = FixedVerdict::Invalid;
    let outcome = world.register("second").await;
    world.registrations.push(outcome);
}

#[when("the customer lists the credentials")]
async fn the_customer_lists(world: &mut CustomerCloudWorld) {
    world.listed = Some(
        ListCloudCredentials::new(world.platform.clone(), Grants::administrator())
            .execute(caller(), world.organisation)
            .await
            .expect("listed"),
    );
}

#[when("the customer creates a customer cloud deployment")]
async fn the_customer_creates(world: &mut CustomerCloudWorld) {
    world.create_on_customer_cloud(ClusterMode::Standard).await;
}

#[when("the customer deletes the credential")]
async fn the_customer_deletes_the_credential(world: &mut CustomerCloudWorld) {
    let id = world.credential.as_ref().expect("a credential").id;
    world.credential_deletion = Some(
        DeleteCloudCredential::new(
            world.store(),
            world.platform.clone(),
            Grants::administrator(),
        )
        .execute(caller(), world.organisation, id)
        .await,
    );
}

#[when(
    expr = "the customer asks for a {string} profile on control plane {string} and node type {string} with {int} to {int} nodes and {int} replicas"
)]
async fn the_customer_asks_for_a_profile(
    world: &mut CustomerCloudWorld,
    mode: String,
    control_plane: String,
    node_type: String,
    min: u8,
    max: u8,
    replicas: u8,
) {
    let outcome = world
        .resolve(spec(
            mode_named(&mode),
            &control_plane,
            &node_type,
            min,
            max,
            replicas,
        ))
        .await;
    if let Ok(Distribution::CustomerCloud { profile, .. }) = &outcome {
        world.profile = Some(profile.clone());
    }
    world.distribution = Some(outcome);
}

#[when(
    expr = "the customer asks the estimate of a {string} profile on control plane {string} and node type {string} with {int} to {int} nodes and {int} replicas"
)]
async fn the_customer_asks_the_estimate(
    world: &mut CustomerCloudWorld,
    mode: String,
    control_plane: String,
    node_type: String,
    min: u8,
    max: u8,
    replicas: u8,
) {
    let catalog = providers(FixedVerdict::Accept);
    let id = world.credential.as_ref().expect("a credential").id;
    world.estimate = Some(
        EstimateClusterCost::new(&catalog, world.platform.clone(), Grants::administrator())
            .execute(
                caller(),
                world.organisation,
                id,
                &Region::new(REGION),
                spec(
                    mode_named(&mode),
                    &control_plane,
                    &node_type,
                    min,
                    max,
                    replicas,
                ),
            )
            .await,
    );
}

#[when(expr = "the customer builds a creation request of kind {string} on the customer cloud")]
async fn the_customer_builds_a_request(world: &mut CustomerCloudWorld, kind: String) {
    world.request = Some(build_request(world, &kind).await);
}

async fn build_request(
    world: &CustomerCloudWorld,
    kind: &str,
) -> Result<CreateDeploymentCommand, DistributionError> {
    let distribution = world
        .resolve(spec_for(ClusterMode::Standard))
        .await
        .expect("a valid distribution");
    CreateDeploymentCommand::new(
        world.organisation,
        DeploymentName("tenant".to_string()),
        kind_named(kind),
        Version::new(26, 0, 1),
        UserId(Uuid::from_u128(9)),
        Environment::Development,
        Region::new(REGION),
        Offer::Sandbox,
    )
    .expect("a creatable name")
    .with_distribution(distribution)
}

#[when("the worker provisions the cluster on a healthy Scaleway")]
async fn the_worker_provisions(world: &mut CustomerCloudWorld) {
    world.provision().await;
}

#[when("the worker runs a second time on a healthy Scaleway")]
async fn the_worker_runs_again(world: &mut CustomerCloudWorld) {
    world.provision().await;
}

#[when("the worker provisions the cluster on a Scaleway that refuses the cluster for quota")]
async fn the_worker_meets_a_quota(world: &mut CustomerCloudWorld) {
    let secret_key = world.secret_key.clone();
    let server = world.server.get().await;
    let refusal = scaleway_api::quota_refusal(server, &secret_key).await;
    let report = worker(
        &server.base_url(),
        &world.platform,
        &world.keys,
        &world.identities,
        &world.runner,
    )
    .provision_pending()
    .await
    .expect("a provisioning pass");
    let hits = refusal.hits().await;
    refusal.remove().await;
    world.provision_reports.push(report);
    world.refusal_hits = Some(hits);
}

#[when("the first heartbeat of the data plane arrives")]
async fn the_first_heartbeat(world: &mut CustomerCloudWorld) {
    world.heartbeat().await;
}

#[when("the deployment comes up")]
fn the_deployment_comes_up(world: &mut CustomerCloudWorld) {
    world.platform.mark_successful(world.deployment().id);
}

#[when("the customer deletes the deployment")]
async fn the_customer_deletes_the_deployment(world: &mut CustomerCloudWorld) {
    let id = world.deployment().id;
    world
        .service()
        .delete_deployment_for_organisation(caller(), world.organisation, id)
        .await
        .expect("deleted");
}

#[when("the worker tears the clusters down")]
async fn the_worker_tears_down(world: &mut CustomerCloudWorld) {
    world.tear_down().await;
}

#[when("the worker tears the clusters down again")]
async fn the_worker_tears_down_again(world: &mut CustomerCloudWorld) {
    world.tear_down().await;
}

#[when("the cluster is deprovisioned directly twice more")]
async fn the_cluster_is_deprovisioned_directly(world: &mut CustomerCloudWorld) {
    let id = world.deployment().dataplane_id;
    let server = world.server.get().await;
    let teardown = scaleway_api::teardown(server).await;
    let provisioner = provisioner(
        &server.base_url(),
        &world.platform,
        &world.keys,
        &world.identities,
        &world.runner,
    );
    let first = provisioner.deprovision(&id).await;
    let second = provisioner.deprovision(&id).await;
    let hits = teardown.hits().await;
    teardown.remove().await;
    world.direct_deprovisions = vec![first, second];
    world.direct_hits = Some(hits);
}

#[when(
    expr = "the customer resizes the cluster to {string} with {int} to {int} nodes and {int} replicas"
)]
async fn the_customer_resizes(
    world: &mut CustomerCloudWorld,
    mode: String,
    min: u8,
    max: u8,
    replicas: u8,
) {
    let deployment_id = world.deployment().id;
    world.plane_before_resize = Some(world.plane());
    let target = spec(mode_named(&mode), MUTUALIZED, SMALL, min, max, replicas);
    let server = world.server.get().await;
    let resizing = scaleway_api::resizing(
        server,
        SMALL,
        MUTUALIZED,
        u32::from(world.profile.as_ref().expect("a profile").min_nodes()),
        &json!({"autoscaling": mode != "dev", "size": min, "min_size": min, "max_size": max}),
    )
    .await;
    let provisioner = provisioner(
        &server.base_url(),
        &world.platform,
        &world.keys,
        &world.identities,
        &world.runner,
    );
    let catalog = providers(FixedVerdict::Accept);
    let outcome = ResizeCustomerCluster::new(
        &catalog,
        &provisioner,
        world.platform.clone(),
        world.platform.clone(),
        world.platform.clone(),
        AllowAll,
        world.audit.clone(),
        Users,
    )
    .execute(member(), world.organisation, deployment_id, target)
    .await;
    let patches = resizing.patches().await;
    resizing.remove().await;
    world.resize_outcome = Some(outcome);
    world.patches = patches;
}

#[then(expr = "the credential is registered for the organisation with the label {string}")]
fn the_credential_is_registered(world: &mut CustomerCloudWorld, label: String) {
    let credential = world.registrations[0].as_ref().expect("registered");
    assert_eq!(credential.organisation_id, world.organisation);
    assert_eq!(credential.provider, Provider::Scaleway);
    assert_eq!(credential.label, label);
    assert!(credential.scope_check.checked_at <= Utc::now());
}

#[then("the credential is listed for the organisation")]
async fn the_credential_is_listed(world: &mut CustomerCloudWorld) {
    let listed = ListCloudCredentials::new(world.platform.clone(), Grants::administrator())
        .execute(caller(), world.organisation)
        .await
        .expect("listed");
    assert_eq!(listed, world.platform.credentials());
    assert_eq!(listed.len(), 1);
    assert_eq!(Some(&listed[0]), world.credential.as_ref());
}

#[then("exactly one sealed secret is stored and it opens back to the secret the customer sent")]
async fn the_sealed_secret_opens(world: &mut CustomerCloudWorld) {
    let sealed = world.platform.sealed();
    assert_eq!(sealed.len(), 1);
    let id = world.credential.as_ref().expect("a credential").id;
    assert_eq!(sealed[0].id, id);
    let opened = world
        .store()
        .get_for_provisioning(&id)
        .await
        .expect("opened");
    assert_eq!(opened.expose(), world.secret());
}

#[then(expr = "the registration is refused for the missing permissions {string}")]
fn refused_for_missing(world: &mut CustomerCloudWorld, missing: String) {
    match world.registrations.last() {
        Some(Err(CoreError::Credential(CredentialError::MissingPermissions {
            missing: listed,
        }))) => assert_eq!(listed, &[missing]),
        other => panic!("expected missing permissions, got {other:?}"),
    }
}

#[then(expr = "the registration is refused for the excess permissions {string}")]
fn refused_for_excess(world: &mut CustomerCloudWorld, extra: String) {
    match world.registrations.last() {
        Some(Err(CoreError::Credential(CredentialError::ExcessPermissions { extra: listed }))) => {
            assert_eq!(listed, &[extra])
        }
        other => panic!("expected excess permissions, got {other:?}"),
    }
}

#[then("nothing was stored for either attempt")]
fn nothing_was_stored(world: &mut CustomerCloudWorld) {
    assert_eq!(world.registrations.len(), 2);
    assert!(world.platform.sealed().is_empty());
    assert!(world.platform.credentials().is_empty());
}

#[then("the plaintext secret is in no response the API would return")]
async fn no_secret_in_responses(world: &mut CustomerCloudWorld) {
    let mut responses = Vec::new();
    for registration in &world.registrations {
        match registration {
            Ok(credential) => responses.push(serde_json::to_string(credential).expect("json")),
            Err(error) => responses.push(format!("{error} {error:?}")),
        }
    }
    responses.push(serde_json::to_string(world.listed.as_ref().expect("a list")).expect("json"));
    responses.push(serde_json::to_string(world.deployment()).expect("json"));
    responses.push(world.provisioning_status().await.to_string());
    assert!(
        matches!(
            world.registrations.last(),
            Some(Err(CoreError::Credential(CredentialError::Invalid)))
        ),
        "{:?}",
        world.registrations.last()
    );
    for response in responses {
        assert!(!response.contains(&world.secret_key), "{response}");
    }
}

#[then("the plaintext secret is in no captured log line")]
fn no_secret_in_logs(world: &mut CustomerCloudWorld) {
    let logs = capture().text();
    assert!(
        logs.contains(&world.plane().id.to_string()),
        "the capture saw nothing of this scenario"
    );
    let leaks: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains(&world.secret_key))
        .collect();
    assert!(leaks.is_empty(), "{leaks:#?}");
    assert!(!logs.contains(&world.secret()));
}

#[then("the plaintext secret is in no stored row or ciphertext")]
fn no_secret_in_storage(world: &mut CustomerCloudWorld) {
    let sealed = world.platform.sealed();
    assert_eq!(sealed.len(), 1);
    for row in &sealed {
        assert!(!row.sealed.ciphertext.is_empty());
        assert!(!contains_bytes(
            &row.sealed.ciphertext,
            world.secret_key.as_bytes()
        ));
        assert!(!contains_bytes(
            &row.sealed.ciphertext,
            world.secret().as_bytes()
        ));
        assert!(!row.sealed.wrapped_dek.as_str().contains(&world.secret_key));
    }
    assert!(!world.platform.dump().contains(&world.secret_key));
}

#[then("the plaintext secret is on no helm command line and in no helm values file")]
fn no_secret_in_helm(world: &mut CustomerCloudWorld) {
    let calls = world.runner.calls();
    assert!(!calls.is_empty());
    for call in calls {
        assert!(call.args.iter().all(|arg| !arg.contains(&world.secret_key)));
        assert!(
            call.values
                .as_deref()
                .is_none_or(|values| !values.contains(&world.secret_key))
        );
    }
}

#[then("the deletion is refused because the credential is in use")]
fn the_deletion_is_refused(world: &mut CustomerCloudWorld) {
    assert!(matches!(
        world.credential_deletion,
        Some(Err(CoreError::Credential(CredentialError::InUse)))
    ));
}

#[then("the credential and its sealed secret are still stored")]
async fn still_stored(world: &mut CustomerCloudWorld) {
    let id = world.credential.as_ref().expect("a credential").id;
    assert_eq!(world.platform.credentials().len(), 1);
    assert_eq!(world.platform.sealed().len(), 1);
    assert!(world.store().get_for_provisioning(&id).await.is_ok());
}

#[then("a second credential used by nothing can be deleted")]
async fn a_spare_can_be_deleted(world: &mut CustomerCloudWorld) {
    let spare = world.register("spare").await.expect("registered");
    assert_eq!(world.platform.sealed().len(), 2);
    DeleteCloudCredential::new(
        world.store(),
        world.platform.clone(),
        Grants::administrator(),
    )
    .execute(caller(), world.organisation, spare.id)
    .await
    .expect("an unused credential is deleted");
    assert_eq!(world.platform.sealed().len(), 1);
}

#[then(expr = "the profile is refused as {string}")]
fn the_profile_is_refused(world: &mut CustomerCloudWorld, name: String) {
    let outcome = world.distribution.as_ref().expect("an outcome");
    assert_eq!(variant(profile_refusal(outcome)), name);
}

#[then(expr = "the refusal reads {string}")]
fn the_refusal_reads(world: &mut CustomerCloudWorld, text: String) {
    let outcome = world.distribution.as_ref().expect("an outcome");
    assert_eq!(profile_refusal(outcome).to_string(), text);
}

#[then("nothing was created")]
fn nothing_was_created(world: &mut CustomerCloudWorld) {
    world.nothing_created();
}

#[then(expr = "the estimate runs from {int} to {int} minor units a month")]
fn the_estimate_runs(world: &mut CustomerCloudWorld, min: u64, max: u64) {
    let estimate = world
        .estimate
        .as_ref()
        .expect("an estimate")
        .as_ref()
        .expect("an accepted profile");
    assert_eq!(estimate.min.minor_units(), min);
    assert_eq!(estimate.max.minor_units(), max);
}

#[then(expr = "no estimate is given and the profile is refused as {string}")]
fn no_estimate(world: &mut CustomerCloudWorld, name: String) {
    match world.estimate.as_ref().expect("an estimate") {
        Err(CoreError::Profile(error)) => assert_eq!(variant(error), name),
        other => panic!("expected a refused profile, got {other:?}"),
    }
}

#[then(expr = "the profile is accepted for the credential with {int} to {int} nodes of {string}")]
fn the_profile_is_accepted(world: &mut CustomerCloudWorld, min: u8, max: u8, node_type: String) {
    let credential = world.credential.as_ref().expect("a credential").id;
    match world.distribution.as_ref().expect("an outcome") {
        Ok(Distribution::CustomerCloud {
            credential_id,
            profile,
        }) => {
            assert_eq!(*credential_id, credential);
            assert_eq!(profile.mode(), ClusterMode::Ha);
            assert_eq!(profile.min_nodes(), min);
            assert_eq!(profile.max_nodes(), max);
            assert_eq!(profile.node_type().as_str(), node_type);
            assert_eq!(profile.node_type().as_str(), MEDIUM);
        }
        other => panic!("expected an accepted profile, got {other:?}"),
    }
}

#[then("the request is refused because a keycloak deployment cannot run in the customer's cloud")]
fn the_request_is_refused(world: &mut CustomerCloudWorld) {
    match world.request.as_ref().expect("a request") {
        Err(error) => assert_eq!(
            error.to_string(),
            "a keycloak deployment cannot run in the customer's cloud"
        ),
        Ok(command) => panic!("the request was accepted: {command:?}"),
    }
}

#[then("no deployment and no data plane were recorded")]
fn nothing_recorded(world: &mut CustomerCloudWorld) {
    world.nothing_created();
}

#[then(expr = "a creation request of kind {string} on the customer cloud is accepted")]
async fn a_request_is_accepted(world: &mut CustomerCloudWorld, kind: String) {
    assert!(build_request(world, &kind).await.is_ok());
}

#[then(
    expr = "the deployment is {string} and its data plane is provisioning without a Herald binding"
)]
fn pending_and_provisioning(world: &mut CustomerCloudWorld, status: String) {
    let deployment = world
        .platform
        .deployment(world.deployment().id)
        .expect("stored");
    assert_eq!(deployment.status.to_string(), status);
    let plane = world.plane();
    assert_eq!(plane.status, DataPlaneStatus::Provisioning);
    assert!(plane.herald.is_none());
    assert!(plane.last_seen_at.is_none());
}

#[then("the create request did not provision anything")]
fn the_create_did_not_provision(world: &mut CustomerCloudWorld) {
    assert_eq!(world.spy.calls(), 0);
    assert!(world.platform.inventory().is_empty());
    assert!(world.runner.calls().is_empty());
    assert!(world.identities.minted().is_empty());
}

#[then("Scaleway was asked for exactly one private network, one cluster and one node pool")]
fn exactly_one_cluster(world: &mut CustomerCloudWorld) {
    assert_eq!(
        world.creation_hits.last(),
        Some(&CreationHits {
            networks: 1,
            clusters: 1,
            pools: 1
        })
    );
    assert_eq!(world.platform.inventory().len(), 3);
}

#[then(
    "the data plane carries the Herald binding minted by the bootstrap and the capacity of the nodes"
)]
fn binding_and_capacity(world: &mut CustomerCloudWorld) {
    let plane = world.plane();
    assert_eq!(
        plane.herald,
        Some(HeraldBinding {
            client_id: format!("herald-{}", plane.id.0),
            subject: format!("subject-{}", plane.id.0),
        })
    );
    let nodes = u32::from(world.deployed_profile().min_nodes());
    assert_eq!(
        plane.capacity,
        Capacity::new(PRO2_S_CPUS * 1000 * nodes, PRO2_S_MEMORY_MIB * nodes, 1).expect("capacity")
    );
}

#[then(expr = "the data plane is still provisioning and its provisioning status is {string}")]
async fn still_provisioning(world: &mut CustomerCloudWorld, status: String) {
    assert_eq!(world.plane().status, DataPlaneStatus::Provisioning);
    assert_eq!(world.provisioning_status().await["status"], json!(status));
}

#[then("the chart prerequisites and then the data plane chart were installed once")]
fn charts_installed_once(world: &mut CustomerCloudWorld) {
    assert_eq!(
        world.runner.releases(),
        ["cnpg", "keda", "eg", "autharie-dataplane"]
    );
    assert_eq!(world.identities.minted(), vec![world.plane().id]);
}

#[then("Scaleway was asked for nothing more and the report says nothing was provisioned")]
fn nothing_more(world: &mut CustomerCloudWorld) {
    assert_eq!(
        world.creation_hits.last(),
        Some(&CreationHits {
            networks: 0,
            clusters: 0,
            pools: 0
        })
    );
    assert_eq!(
        world.provision_reports.last(),
        Some(&ProvisionReport::default())
    );
}

#[then(expr = "the data plane is active and its provisioning status is {string}")]
async fn active_and_ready(world: &mut CustomerCloudWorld, status: String) {
    let plane = world.plane();
    assert_eq!(plane.status, DataPlaneStatus::Active);
    assert!(plane.last_seen_at.is_some());
    assert_eq!(world.provisioning_status().await["status"], json!(status));
}

#[then(expr = "the provisioning step is {string}")]
async fn the_provisioning_step_is(world: &mut CustomerCloudWorld, wanted: String) {
    let view = world.provisioning_status().await;
    match wanted.as_str() {
        "none" => assert!(view.get("step").is_none(), "{view}"),
        _ => assert_eq!(view["step"], json!(wanted)),
    }
}

#[then("the worker reports one failed provisioning")]
fn one_failed(world: &mut CustomerCloudWorld) {
    assert_eq!(
        world.provision_reports.last(),
        Some(&ProvisionReport {
            provisioned: 0,
            failed: 1,
            deferred: 0
        })
    );
}

#[then(expr = "the data plane is failed with the reason {string}")]
fn failed_with_reason(world: &mut CustomerCloudWorld, reason: String) {
    let plane = world.plane();
    assert_eq!(plane.status, DataPlaneStatus::Failed);
    assert_eq!(plane.failure_reason.as_deref(), Some(reason.as_str()));
}

#[then(expr = "the provisioning status is {string} with that reason")]
async fn failed_status_with_reason(world: &mut CustomerCloudWorld, status: String) {
    let view = world.provisioning_status().await;
    assert_eq!(view["status"], json!(status));
    assert_eq!(
        view["failure_reason"],
        json!(world.plane().failure_reason.expect("a reason"))
    );
}

#[then("everything the failed attempt created was released")]
fn failed_attempt_released(world: &mut CustomerCloudWorld) {
    let inventory = world.platform.inventory();
    assert!(!inventory.is_empty());
    assert!(inventory.iter().all(|(_, released)| *released));
    assert_eq!(
        world.refusal_hits,
        Some(RefusalHits {
            cluster_attempts: 1,
            network_deletions: 1
        })
    );
}

#[then("nothing was bootstrapped")]
fn nothing_bootstrapped(world: &mut CustomerCloudWorld) {
    assert!(world.runner.calls().is_empty());
    assert!(world.identities.minted().is_empty());
}

#[then("the kubeconfig the chart installs saw is the one Scaleway served")]
fn the_kubeconfig_was_served(world: &mut CustomerCloudWorld) {
    let calls = world.runner.calls();
    assert_eq!(calls.len(), 4);
    for call in calls {
        assert_eq!(call.kubeconfig, kubeconfig(&world.kubeconfig_token));
    }
}

#[then("the kubeconfig files no longer exist once the bootstrap returned")]
fn the_kubeconfig_is_gone(world: &mut CustomerCloudWorld) {
    for call in world.runner.calls() {
        assert!(!call.dir.exists(), "{} was left behind", call.dir.display());
    }
}

#[then("the kubeconfig is on no helm command line and in no values file")]
fn the_kubeconfig_is_not_passed(world: &mut CustomerCloudWorld) {
    for call in world.runner.calls() {
        assert!(
            call.args
                .iter()
                .all(|arg| !arg.contains(&world.kubeconfig_token))
        );
        assert!(
            call.values
                .as_deref()
                .is_none_or(|values| !values.contains(&world.kubeconfig_token))
        );
    }
}

#[then("the kubeconfig is in no log line and in no stored state")]
fn the_kubeconfig_is_not_kept(world: &mut CustomerCloudWorld) {
    let logs = capture().text();
    assert!(logs.contains(&world.plane().id.to_string()));
    assert!(!logs.contains(&world.kubeconfig_token));
    assert!(!world.platform.dump().contains(&world.kubeconfig_token));
}

#[then("Scaleway received exactly one deletion of the pool, the cluster and the private network")]
fn one_deletion_each(world: &mut CustomerCloudWorld) {
    assert_eq!(
        world.teardown_hits.first(),
        Some(&TeardownHits {
            pools: 1,
            clusters: 1,
            networks: 1
        })
    );
}

#[then("the inventory shows nothing left")]
fn nothing_left(world: &mut CustomerCloudWorld) {
    let inventory = world.platform.inventory();
    assert_eq!(inventory.len(), 3);
    assert!(inventory.iter().all(|(_, released)| *released));
}

#[then("the Herald identity of the data plane was revoked once")]
fn revoked_once(world: &mut CustomerCloudWorld) {
    assert_eq!(world.identities.revoked(), vec![world.plane().id]);
}

#[then(expr = "the data plane is disabled and the deployment is {string}")]
fn disabled_and_deleted(world: &mut CustomerCloudWorld, status: String) {
    assert_eq!(world.plane().status, DataPlaneStatus::Disabled);
    let deployment = world
        .platform
        .deployment(world.deployment().id)
        .expect("stored");
    assert_eq!(deployment.status.to_string(), status);
    assert!(deployment.deleted_at.is_some());
}

#[then("the second pass removed and revoked nothing and left Scaleway untouched")]
fn the_second_pass_did_nothing(world: &mut CustomerCloudWorld) {
    assert_eq!(world.teardown_reports.len(), 2);
    assert_eq!(world.teardown_reports[0].released, 1);
    assert_eq!(world.teardown_reports[0].confirmed, 1);
    assert_eq!(world.teardown_reports[1], TeardownReport::default());
    assert_eq!(
        world.teardown_hits[1],
        TeardownHits {
            pools: 0,
            clusters: 0,
            networks: 0
        }
    );
    assert_eq!(world.identities.revoked().len(), 1);
}

#[then("both calls succeeded and Scaleway received no deletion")]
fn the_direct_calls_removed_nothing(world: &mut CustomerCloudWorld) {
    assert_eq!(world.direct_deprovisions.len(), 2);
    assert!(world.direct_deprovisions.iter().all(Result::is_ok));
    assert_eq!(
        world.direct_hits,
        Some(TeardownHits {
            pools: 0,
            clusters: 0,
            networks: 0
        })
    );
}

#[then(
    expr = "the resize is accepted with the profile {string}, {int} nodes at least and {int} replicas"
)]
fn the_resize_is_accepted(world: &mut CustomerCloudWorld, mode: String, nodes: u8, replicas: u8) {
    let after = match world.resize_outcome.as_ref().expect("a resize") {
        Ok(profile) => profile,
        Err(error) => panic!("expected an accepted resize, got {error:?}"),
    };
    let before = world.profile.as_ref().expect("a profile");
    assert_eq!(after.mode(), mode_named(&mode));
    assert_eq!(after.min_nodes(), nodes);
    assert_eq!(after.min_nodes(), before.min_nodes() + 1);
    assert_eq!(after.replication().get(), replicas);
    assert_eq!(after.replication().get(), before.replication().get() + 1);
    assert_eq!(after.node_type(), before.node_type());
    assert_eq!(after.control_plane(), before.control_plane());
}

#[then("Scaleway received exactly one node pool patch moving the pool to 2 nodes with autoscaling")]
fn one_pool_patch(world: &mut CustomerCloudWorld) {
    assert!(
        matches!(world.resize_outcome, Some(Ok(_))),
        "{:?}",
        world.resize_outcome
    );
    assert_eq!(world.patches, 1);
}

#[then("Scaleway received no node pool patch")]
fn no_pool_patch(world: &mut CustomerCloudWorld) {
    assert_eq!(world.patches, 0);
}

#[then("the data plane is the same one, still active, with the same Herald binding")]
fn same_data_plane(world: &mut CustomerCloudWorld) {
    let before = world.plane_before_resize.as_ref().expect("a plane");
    let after = world.plane();
    assert_eq!(before.status, DataPlaneStatus::Active);
    assert_eq!(&after, before);
    assert_eq!(world.platform.planes().len(), 1);
    assert_eq!(
        world
            .platform
            .deployment(world.deployment().id)
            .expect("stored")
            .dataplane_id,
        before.id
    );
}

#[then(expr = "the resize is refused with {int} replicas in use and {int} allowed")]
fn the_resize_is_refused(world: &mut CustomerCloudWorld, in_use: u8, allowed: u8) {
    match world.resize_outcome.as_ref().expect("a resize") {
        Err(CoreError::Resize(ResizeError::ReplicasExceedTarget {
            target,
            in_use: seen,
            allowed: limit,
        })) => {
            assert_eq!(*target, ClusterMode::Dev);
            assert_eq!(*seen, in_use);
            assert_eq!(*limit, allowed);
        }
        other => panic!("expected a refused resize, got {other:?}"),
    }
}

#[then(expr = "the persisted profile is {string} with {int} to {int} nodes and {int} replicas")]
fn the_persisted_profile(
    world: &mut CustomerCloudWorld,
    mode: String,
    min: u8,
    max: u8,
    replicas: u8,
) {
    let stored = world
        .platform
        .deployment(world.deployment().id)
        .expect("stored");
    let Distribution::CustomerCloud { profile, .. } = stored.distribution else {
        panic!("not a customer cloud deployment");
    };
    assert_eq!(profile.mode(), mode_named(&mode));
    assert_eq!(profile.min_nodes(), min);
    assert_eq!(profile.max_nodes(), max);
    assert_eq!(profile.replication().get(), replicas);
}

#[then("one audit entry records the change of profile")]
fn one_audit_entry(world: &mut CustomerCloudWorld) {
    let entries = world.audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action.0, "deployment.cluster_profile.updated");
    assert!(entries[0].change.is_some());
}

#[then("no audit entry was recorded")]
fn no_audit_entry(world: &mut CustomerCloudWorld) {
    assert!(world.audit.entries().is_empty());
}
