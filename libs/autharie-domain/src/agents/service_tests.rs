use std::sync::{Arc, Mutex};

use autharie_auth::Identity;
use chrono::{DateTime, DurationRound, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;
use crate::{
    CoreError,
    action::{Action, ports::MockActionRepository},
    agents::{
        ports::{AgentAccess, AgentOutcomeService, AgentRepository, AgentService},
        registry::{
            AccessLists, AgentClientId, AutonomousAgent, CredentialRequest, CredentialState,
            ReportedRegistry, SealedCredential,
        },
        registry_ports::AgentRegistryRepository,
        server_ports::AgentServerRepository,
        servers::{CustomerServer, Discovery, ServerId},
        service::{AgentOutcomeServiceImpl, AgentPublisher, AgentServiceImpl},
    },
    audit::{AuditEntry, ports::MockAuditRepository},
    dataplane::{
        entities::DataPlane,
        ports::MockDataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::MockDeploymentRepository,
    },
    organisation::{
        Organisation, OrganisationId,
        ports::MockOrganisationRepository,
        value_objects::{OrganisationName, OrganisationSlug, Plan},
    },
    user::{User, UserId, ports::UserRepository},
    version::Version,
};

pub(super) const ORGANISATION: Uuid = Uuid::from_u128(1);
pub(super) const DEPLOYMENT: Uuid = Uuid::from_u128(2);
pub(super) const DATAPLANE: Uuid = Uuid::from_u128(3);
pub(super) const PERMIT: &str = "permit(principal, action, resource);";

#[derive(Clone)]
pub(super) struct StoredRequest {
    pub(super) request: CredentialRequest,
    pub(super) sealed: Option<String>,
    pub(super) delivered: bool,
}

#[derive(Default)]
pub(super) struct State {
    settings: Option<AgentSettings>,
    policies: Vec<AgentPolicy>,
    pub(super) access: AccessLists,
    pub(super) registry: ReportedRegistry,
    pub(super) autonomous: Vec<AutonomousAgent>,
    pub(super) requests: Vec<StoredRequest>,
    servers: Vec<CustomerServer>,
}

#[derive(Clone, Default)]
pub(super) struct Memory(pub(super) Arc<Mutex<State>>);

impl Memory {
    pub(super) fn read<T>(&self, read: impl FnOnce(&State) -> T) -> T {
        read(&self.0.lock().unwrap())
    }

    fn settings_row(state: &mut State, deployment_id: DeploymentId) -> &mut AgentSettings {
        state
            .settings
            .get_or_insert_with(|| AgentSettings::unset(deployment_id))
    }
}

impl AgentRepository for Memory {
    async fn settings(&self, _: DeploymentId) -> Result<Option<AgentSettings>, CoreError> {
        Ok(self.read(|state| state.settings.clone()))
    }

    async fn save_settings(
        &self,
        deployment_id: DeploymentId,
        input: &AgentSettingsInput,
        _: DateTime<Utc>,
    ) -> Result<AgentSettings, CoreError> {
        let mut state = self.0.lock().unwrap();
        let row = Self::settings_row(&mut state, deployment_id);
        row.enabled = input.enabled;
        row.realm = input.realm.clone();
        row.revision += 1;
        Ok(row.clone())
    }

    async fn next_revision(
        &self,
        deployment_id: DeploymentId,
        _: DateTime<Utc>,
    ) -> Result<u64, CoreError> {
        let mut state = self.0.lock().unwrap();
        let row = Self::settings_row(&mut state, deployment_id);
        row.revision += 1;
        Ok(row.revision)
    }

    async fn record_status(
        &self,
        deployment_id: DeploymentId,
        revision: u64,
        status: &AgentStatus,
        mcp_url: Option<&str>,
    ) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        let row = Self::settings_row(&mut state, deployment_id);
        if row.applied_revision > revision {
            return Ok(false);
        }
        row.applied_revision = revision;
        row.status = Some(status.clone());
        row.mcp_url = mcp_url.map(str::to_string);
        Ok(true)
    }

    async fn list_policies(&self, _: DeploymentId) -> Result<Vec<AgentPolicy>, CoreError> {
        Ok(self.read(|state| state.policies.clone()))
    }

    async fn get_policy(
        &self,
        _: DeploymentId,
        id: PolicyId,
    ) -> Result<Option<AgentPolicy>, CoreError> {
        Ok(self.read(|state| state.policies.iter().find(|p| p.id == id).cloned()))
    }

    async fn insert_policy(&self, policy: &AgentPolicy) -> Result<(), CoreError> {
        self.0.lock().unwrap().policies.push(policy.clone());
        Ok(())
    }

    async fn update_policy(&self, policy: &AgentPolicy) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        match state.policies.iter_mut().find(|p| p.id == policy.id) {
            Some(slot) => {
                *slot = policy.clone();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete_policy(&self, _: DeploymentId, id: PolicyId) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        let before = state.policies.len();
        state.policies.retain(|p| p.id != id);
        Ok(state.policies.len() != before)
    }

    async fn record_policy_outcome(
        &self,
        _: DeploymentId,
        id: PolicyId,
        revision: u64,
        phase: PolicyPhase,
        message: Option<&str>,
        _: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        match state
            .policies
            .iter_mut()
            .find(|p| p.id == id && p.desired_revision <= revision)
        {
            Some(policy) => {
                policy.phase = Some(phase);
                policy.message = message.map(str::to_string);
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

impl AgentRegistryRepository for Memory {
    async fn access(&self, _: DeploymentId) -> Result<AccessLists, CoreError> {
        Ok(self.read(|state| state.access.clone()))
    }

    async fn save_access(
        &self,
        _: DeploymentId,
        access: &AccessLists,
        _: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.0.lock().unwrap().access = access.clone();
        Ok(())
    }

    async fn registry(&self, _: DeploymentId) -> Result<ReportedRegistry, CoreError> {
        Ok(self.read(|state| state.registry.clone()))
    }

    async fn replace_registry(
        &self,
        _: DeploymentId,
        registry: &ReportedRegistry,
        _: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.0.lock().unwrap().registry = registry.clone();
        Ok(())
    }

    async fn autonomous_agents(&self, _: DeploymentId) -> Result<Vec<AutonomousAgent>, CoreError> {
        Ok(self.read(|state| state.autonomous.clone()))
    }

    async fn insert_autonomous_agent(
        &self,
        _: DeploymentId,
        agent: &AutonomousAgent,
    ) -> Result<(), CoreError> {
        self.0.lock().unwrap().autonomous.push(agent.clone());
        Ok(())
    }

    async fn delete_autonomous_agent(
        &self,
        _: DeploymentId,
        client_id: &AgentClientId,
    ) -> Result<(), CoreError> {
        self.0
            .lock()
            .unwrap()
            .autonomous
            .retain(|agent| &agent.client_id != client_id);
        Ok(())
    }

    async fn insert_request(
        &self,
        _: DeploymentId,
        request: &CredentialRequest,
        _: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.0.lock().unwrap().requests.push(StoredRequest {
            request: request.clone(),
            sealed: None,
            delivered: false,
        });
        Ok(())
    }

    async fn open_requests(
        &self,
        _: DeploymentId,
        now: DateTime<Utc>,
    ) -> Result<Vec<CredentialRequest>, CoreError> {
        Ok(self.read(|state| {
            state
                .requests
                .iter()
                .filter(|stored| {
                    stored.sealed.is_none() && !stored.delivered && stored.request.expires_at > now
                })
                .map(|stored| stored.request.clone())
                .collect()
        }))
    }

    async fn store_sealed(
        &self,
        _: DeploymentId,
        request_id: Uuid,
        client_id: &AgentClientId,
        sealed: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        match state.requests.iter_mut().find(|stored| {
            stored.request.request_id == request_id
                && &stored.request.client_id == client_id
                && stored.sealed.is_none()
                && !stored.delivered
                && stored.request.expires_at > now
        }) {
            Some(stored) => {
                stored.sealed = Some(sealed.to_string());
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn credential_state(
        &self,
        _: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<CredentialState, CoreError> {
        Ok(self.read(|state| {
            match state
                .requests
                .iter()
                .find(|stored| stored.request.request_id == request_id)
                .filter(|stored| stored.request.expires_at > now)
            {
                None => CredentialState::Missing,
                Some(stored) if stored.delivered => CredentialState::Delivered,
                Some(stored) if stored.sealed.is_none() => CredentialState::Pending,
                Some(_) => CredentialState::Ready,
            }
        }))
    }

    async fn take_sealed(
        &self,
        _: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<SealedCredential>, CoreError> {
        let mut state = self.0.lock().unwrap();
        let Some(stored) = state.requests.iter_mut().find(|stored| {
            stored.request.request_id == request_id
                && !stored.delivered
                && stored.sealed.is_some()
                && stored.request.expires_at > now
        }) else {
            return Ok(None);
        };
        stored.delivered = true;
        Ok(stored.sealed.take().map(|sealed| SealedCredential {
            client_id: stored.request.client_id.clone(),
            sealed,
        }))
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64, CoreError> {
        let mut state = self.0.lock().unwrap();
        let before = state.requests.len();
        state
            .requests
            .retain(|stored| stored.request.expires_at > now);
        Ok((before - state.requests.len()) as u64)
    }
}

impl AgentServerRepository for Memory {
    async fn list_servers(&self, _: DeploymentId) -> Result<Vec<CustomerServer>, CoreError> {
        Ok(self.read(|state| state.servers.clone()))
    }

    async fn get_server(
        &self,
        _: DeploymentId,
        id: ServerId,
    ) -> Result<Option<CustomerServer>, CoreError> {
        Ok(self.read(|state| state.servers.iter().find(|s| s.id == id).cloned()))
    }

    async fn insert_server(&self, server: &CustomerServer) -> Result<(), CoreError> {
        let mut state = self.0.lock().unwrap();
        if state.servers.iter().any(|s| s.slug == server.slug) {
            return Err(AgentError::ServerSlugTaken {
                slug: server.slug.to_string(),
            }
            .into());
        }
        state.servers.push(server.clone());
        Ok(())
    }

    async fn update_server(&self, server: &CustomerServer) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        match state.servers.iter_mut().find(|s| s.id == server.id) {
            Some(slot) => {
                *slot = server.clone();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete_server(&self, _: DeploymentId, id: ServerId) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        let before = state.servers.len();
        state.servers.retain(|s| s.id != id);
        Ok(state.servers.len() != before)
    }

    async fn record_discovery(
        &self,
        _: DeploymentId,
        id: ServerId,
        discovery: &Discovery,
        answers: Option<DateTime<Utc>>,
    ) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        let Some(server) = state.servers.iter_mut().find(|s| s.id == id) else {
            return Ok(false);
        };
        let answered = answers.unwrap_or(discovery.discovered_at);
        let fresh = server.discovery_requested_at.is_none_or(|asked| {
            answered
                >= asked
                    .duration_trunc(chrono::Duration::milliseconds(1))
                    .unwrap()
        }) && server
            .discovery
            .as_ref()
            .is_none_or(|known| discovery.discovered_at > known.discovered_at);
        if fresh {
            server.discovery = Some(discovery.clone());
        }
        Ok(fresh)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Access {
    pub(super) manage: bool,
}

impl AgentAccess for Access {
    async fn can_view_agents(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        Ok(())
    }

    async fn can_manage_agents(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        if self.manage {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied {
                reason: "insufficient permissions".to_string(),
            })
        }
    }
}

pub(super) struct Users;

impl UserRepository for Users {
    async fn upsert_by_email(&self, user: &User) -> Result<User, CoreError> {
        Ok(user.clone())
    }

    async fn find_by_sub(&self, sub: &str) -> Result<Option<User>, CoreError> {
        Ok(Some(User {
            id: UserId(Uuid::from_u128(9)),
            email: "a@b.c".to_string(),
            name: "a".to_string(),
            sub: sub.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    async fn find_by_email(&self, _: &str) -> Result<Option<User>, CoreError> {
        Ok(None)
    }
}

pub(super) fn caller() -> Identity {
    Identity::User(autharie_auth::User {
        scopes: Vec::new(),
        act: None,
        id: "user".to_string(),
        username: "user".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

fn deployment(kind: DeploymentKind) -> Deployment {
    let at = Utc::now();
    Deployment {
        topology: Default::default(),
        id: DeploymentId(DEPLOYMENT),
        organisation_id: OrganisationId(ORGANISATION),
        dataplane_id: DataPlaneId(DATAPLANE),
        name: DeploymentName("auth".to_string()),
        kind,
        version: Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: "production-auth".to_string(),
        environment: crate::deployments::environment::Environment::Development,
        offer: None,
        restored_from: None,
        resources: DeploymentResources::DEFAULT,
        created_by: UserId(Uuid::from_u128(4)),
        created_at: at,
        updated_at: at,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: Default::default(),
        maintenance_window: None,
        network_access: crate::deployments::network::NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Written {
    pub(super) actions: Arc<Mutex<Vec<Action>>>,
    pub(super) audit: Arc<Mutex<Vec<AuditEntry>>>,
}

impl Written {
    pub(super) fn payloads(&self) -> Vec<Value> {
        self.actions
            .lock()
            .unwrap()
            .iter()
            .filter(|action| action.action_type.0 == AGENTS_ACTION_TYPE)
            .map(|action| action.payload.data.clone())
            .collect()
    }

    pub(super) fn audited(&self) -> Vec<String> {
        self.audit
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.action.0.clone())
            .collect()
    }
}

type Service = AgentServiceImpl<
    MockDeploymentRepository,
    MockOrganisationRepository,
    Memory,
    MockActionRepository,
    MockAuditRepository,
    Users,
    Access,
>;

struct World {
    service: Service,
    memory: Memory,
    written: Written,
}

pub(super) fn deployments_of(kind: DeploymentKind) -> MockDeploymentRepository {
    let mut deployments = MockDeploymentRepository::new();
    deployments.expect_get_by_id().returning(move |_| {
        let found = deployment(kind.clone());
        Box::pin(async move { Ok(Some(found)) })
    });
    deployments
}

pub(super) fn actions_into(written: &Written) -> MockActionRepository {
    let mut actions = MockActionRepository::new();
    let held = Arc::clone(&written.actions);
    actions.expect_append().returning(move |action| {
        held.lock().unwrap().push(action);
        Box::pin(async { Ok(()) })
    });
    actions
}

pub(super) fn organisation(mcp_enabled: bool) -> Organisation {
    let mut organisation = Organisation::new(
        OrganisationName::new("acme").unwrap(),
        OrganisationSlug::new("acme").unwrap(),
        UserId(Uuid::from_u128(4)),
        Plan::Starter,
    );
    organisation.id = OrganisationId(ORGANISATION);
    organisation.set_mcp_enabled(mcp_enabled);
    organisation
}

fn world(kind: DeploymentKind, mcp_enabled: bool, manage: bool, domain: Option<&str>) -> World {
    let memory = Memory::default();
    let written = Written::default();

    let mut organisations = MockOrganisationRepository::new();
    organisations.expect_find_by_id().returning(move |_| {
        let found = organisation(mcp_enabled);
        Box::pin(async move { Ok(Some(found)) })
    });

    let mut audit = MockAuditRepository::new();
    let held = Arc::clone(&written.audit);
    audit.expect_append().returning(move |entry| {
        held.lock().unwrap().push(entry);
        Box::pin(async { Ok(()) })
    });

    World {
        service: AgentServiceImpl::new(
            deployments_of(kind),
            organisations,
            memory.clone(),
            actions_into(&written),
            audit,
            Users,
            Access { manage },
            domain.map(str::to_string),
        ),
        memory,
        written,
    }
}

fn ferriskey(mcp_enabled: bool) -> World {
    world(DeploymentKind::Ferriskey, mcp_enabled, true, None)
}

pub(super) fn org() -> OrganisationId {
    OrganisationId(ORGANISATION)
}

pub(super) fn dep() -> DeploymentId {
    DeploymentId(DEPLOYMENT)
}

pub(super) fn settings(enabled: bool) -> AgentSettingsInput {
    AgentSettingsInput::parse(enabled, Some("acme")).unwrap()
}

fn policy_input(name: &str, enabled: bool) -> PolicyInput {
    PolicyInput::parse(
        name,
        None,
        enabled,
        Some(json!({ "effect": "permit" })),
        PERMIT,
    )
    .unwrap()
}

async fn enable(world: &World) {
    world
        .service
        .set_agents(caller(), org(), dep(), settings(true))
        .await
        .expect("saved");
}

async fn add(world: &World, name: &str, enabled: bool) -> AgentPolicy {
    world
        .service
        .create_agent_policy(caller(), org(), dep(), policy_input(name, enabled))
        .await
        .expect("created")
}

pub(super) fn ingest()
-> AgentOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory> {
    ingest_on(&Memory::default(), DATAPLANE)
}

pub(super) fn ingest_on(
    memory: &Memory,
    speaking: Uuid,
) -> AgentOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory> {
    let mut dataplanes = MockDataPlaneRepository::new();
    dataplanes
        .expect_find_by_herald_subject()
        .returning(move |_| {
            let mut dataplane = DataPlane::new(
                DataPlaneAllocation::Shared,
                Region::new("somewhere"),
                Capacity::new(1000, 1024, 10).unwrap(),
            );
            dataplane.id = DataPlaneId(speaking);
            Box::pin(async move { Ok(Some(dataplane)) })
        });

    AgentOutcomeServiceImpl::new(
        deployments_of(DeploymentKind::Ferriskey),
        dataplanes,
        memory.clone(),
    )
}

pub(super) fn outcome(
    revision: u64,
    phase: &str,
    policies: Vec<(PolicyId, &str, Option<&str>)>,
) -> AgentsOutcome {
    AgentsOutcome::parse(
        revision,
        phase,
        true,
        Some("https://reported.example/mcp".to_string()),
        None,
        policies
            .into_iter()
            .map(|(id, phase, message)| (id.0, phase.to_string(), message.map(str::to_string)))
            .collect(),
    )
    .unwrap()
}

#[tokio::test]
async fn saving_the_settings_sends_the_action_with_the_frozen_fields() {
    let world = ferriskey(true);

    enable(&world).await;

    let payloads = world.written.payloads();
    assert_eq!(payloads.len(), 1);
    assert_eq!(
        payloads[0],
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "production-auth",
            "enabled": true,
            "realm": "acme",
            "revision": 1,
            "policies": [],
            "access": { "approval_required": false, "approved": [], "blocked": [] },
            "autonomous": [],
            "deleted_agents": [],
            "servers": [],
        })
    );
    assert_eq!(world.written.audited(), vec![AGENTS_UPDATED_AUDIT]);
}

#[tokio::test]
async fn agents_stay_off_in_the_action_while_the_organisation_switch_is_off() {
    let world = ferriskey(false);

    enable(&world).await;

    let payloads = world.written.payloads();
    assert_eq!(payloads[0]["enabled"], false);
    let view = world
        .service
        .get_agents(caller(), org(), dep())
        .await
        .unwrap();
    assert!(view.enabled);
    assert!(!view.organisation_mcp_enabled);
}

#[tokio::test]
async fn every_change_bumps_the_revision_and_sends_only_enabled_policies() {
    let world = ferriskey(true);
    enable(&world).await;

    let kept = add(&world, "kept", true).await;
    let off = add(&world, "off", false).await;
    world
        .service
        .update_agent_policy(
            caller(),
            org(),
            dep(),
            kept.id,
            policy_input("kept-2", true),
        )
        .await
        .unwrap();
    world
        .service
        .delete_agent_policy(caller(), org(), dep(), off.id)
        .await
        .unwrap();

    let payloads = world.written.payloads();
    let revisions: Vec<u64> = payloads
        .iter()
        .map(|payload| payload["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, vec![1, 2, 3, 4, 5]);

    assert_eq!(payloads[2]["policies"].as_array().unwrap().len(), 1);
    let last = payloads.last().unwrap();
    assert_eq!(
        last["policies"],
        json!([{ "id": kept.id.0, "name": "kept-2", "cedar": PERMIT }])
    );
}

#[tokio::test]
async fn a_new_policy_waits_as_pending_and_a_disabled_one_has_no_phase() {
    let world = ferriskey(true);

    let enabled = add(&world, "on", true).await;
    let disabled = add(&world, "off", false).await;

    assert_eq!(enabled.phase, Some(PolicyPhase::Pending));
    assert_eq!(disabled.phase, None);
    assert_eq!(enabled.rule, Some(json!({ "effect": "permit" })));
}

#[test]
fn cedar_that_does_not_parse_is_refused_before_anything_is_stored() {
    let error = PolicyInput::parse("p", None, true, None, "permit(").expect_err("refused");

    assert!(matches!(error, AgentError::InvalidCedar { .. }));
}

#[tokio::test]
async fn only_a_member_who_manages_instances_changes_agents() {
    let world = world(DeploymentKind::Ferriskey, true, false, None);

    let set = world
        .service
        .set_agents(caller(), org(), dep(), settings(true))
        .await;
    let created = world
        .service
        .create_agent_policy(caller(), org(), dep(), policy_input("p", true))
        .await;
    let read = world
        .service
        .list_agent_policies(caller(), org(), dep())
        .await;

    assert!(matches!(set, Err(CoreError::PermissionDenied { .. })));
    assert!(matches!(created, Err(CoreError::PermissionDenied { .. })));
    assert!(read.is_ok());
    assert!(world.written.payloads().is_empty());
}

#[tokio::test]
async fn a_keycloak_deployment_cannot_have_agents_but_can_be_read() {
    let world = world(DeploymentKind::Keycloak, true, true, None);

    let set = world
        .service
        .set_agents(caller(), org(), dep(), settings(true))
        .await;
    let read = world.service.get_agents(caller(), org(), dep()).await;

    assert!(matches!(
        set,
        Err(CoreError::Agent(AgentError::UnsupportedKind))
    ));
    assert!(!read.unwrap().enabled);
}

#[tokio::test]
async fn an_unknown_policy_is_not_found() {
    let world = ferriskey(true);

    let result = world
        .service
        .delete_agent_policy(caller(), org(), dep(), PolicyId(Uuid::from_u128(77)))
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Agent(AgentError::PolicyNotFound { .. }))
    ));
}

#[tokio::test]
async fn the_mcp_url_comes_from_the_deployment_host() {
    let world = world(DeploymentKind::Ferriskey, true, true, Some("autharie.fr"));

    let view = world
        .service
        .get_agents(caller(), org(), dep())
        .await
        .unwrap();

    assert_eq!(
        view.mcp_url.as_deref(),
        Some("https://auth.autharie.fr/mcp")
    );
    assert_eq!(view.realm, "master");
    assert_eq!(view.status, None);
}

#[tokio::test]
async fn the_mcp_url_the_data_plane_reports_wins_over_the_guess() {
    let world = world(DeploymentKind::Ferriskey, true, true, Some("autharie.fr"));
    enable(&world).await;
    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), outcome(1, "Ready", vec![]))
        .await
        .unwrap();

    let view = world
        .service
        .get_agents(caller(), org(), dep())
        .await
        .unwrap();

    assert_eq!(
        view.mcp_url.as_deref(),
        Some("https://reported.example/mcp")
    );
}

#[tokio::test]
async fn an_outcome_sets_the_status_and_each_policys_phase() {
    let world = ferriskey(true);
    enable(&world).await;
    let good = add(&world, "good", true).await;
    let bad = add(&world, "bad", true).await;

    let changed = ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            outcome(
                3,
                "Ready",
                vec![
                    (good.id, "Applied", None),
                    (bad.id, "Invalid", Some("unexpected token")),
                ],
            ),
        )
        .await
        .unwrap();

    assert!(changed);
    let status = world
        .memory
        .read(|state| state.settings.clone().unwrap().status.unwrap());
    assert_eq!(status.phase, AgentsPhase::Ready);
    assert!(status.gateway_client_ready);
    let phases: Vec<_> = world.memory.read(|state| {
        state
            .policies
            .iter()
            .map(|p| (p.phase, p.message.clone()))
            .collect()
    });
    assert_eq!(
        phases,
        vec![
            (Some(PolicyPhase::Applied), None),
            (
                Some(PolicyPhase::Invalid),
                Some("unexpected token".to_string())
            )
        ]
    );
}

#[tokio::test]
async fn a_stale_outcome_changes_nothing() {
    let world = ferriskey(true);
    enable(&world).await;
    let policy = add(&world, "p", true).await;
    let service = ingest_on(&world.memory, DATAPLANE);
    service
        .report_agents_outcome(
            caller(),
            dep(),
            outcome(2, "Ready", vec![(policy.id, "Applied", None)]),
        )
        .await
        .unwrap();

    let changed = service
        .report_agents_outcome(
            caller(),
            dep(),
            outcome(1, "Failed", vec![(policy.id, "Failed", Some("old"))]),
        )
        .await
        .unwrap();

    assert!(!changed);
    let (status, phase) = world.memory.read(|state| {
        (
            state.settings.clone().unwrap().status.unwrap(),
            state.policies[0].phase,
        )
    });
    assert_eq!(status.phase, AgentsPhase::Ready);
    assert_eq!(phase, Some(PolicyPhase::Applied));
}

#[tokio::test]
async fn an_outcome_older_than_a_policys_own_revision_leaves_that_policy_alone() {
    let world = ferriskey(true);
    enable(&world).await;
    let first = add(&world, "first", true).await;
    let second = add(&world, "second", true).await;

    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            outcome(
                2,
                "Ready",
                vec![(first.id, "Applied", None), (second.id, "Applied", None)],
            ),
        )
        .await
        .unwrap();

    let phases: Vec<_> = world
        .memory
        .read(|state| state.policies.iter().map(|p| p.phase).collect());
    assert_eq!(
        phases,
        vec![Some(PolicyPhase::Applied), Some(PolicyPhase::Pending)]
    );
}

#[tokio::test]
async fn an_outcome_from_the_future_or_for_unknown_agents_is_ignored() {
    let world = ferriskey(true);
    enable(&world).await;

    let future = ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), outcome(9, "Ready", vec![]))
        .await
        .unwrap();
    let nothing = ingest()
        .report_agents_outcome(caller(), dep(), outcome(1, "Ready", vec![]))
        .await
        .unwrap();

    assert!(!future);
    assert!(!nothing);
}

#[tokio::test]
async fn another_data_planes_outcome_is_refused() {
    let world = ferriskey(true);
    enable(&world).await;

    let result = ingest_on(&world.memory, Uuid::from_u128(4))
        .report_agents_outcome(caller(), dep(), outcome(1, "Ready", vec![]))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn the_switch_resends_every_deployment_that_has_agent_settings() {
    let world = ferriskey(true);
    enable(&world).await;
    add(&world, "p", true).await;

    let mut deployments = MockDeploymentRepository::new();
    deployments.expect_list_by_organisation().returning(|_| {
        let mut gone = deployment(DeploymentKind::Ferriskey);
        gone.deleted_at = Some(Utc::now());
        let all = vec![
            deployment(DeploymentKind::Ferriskey),
            deployment(DeploymentKind::Keycloak),
            gone,
        ];
        Box::pin(async move { Ok(all) })
    });
    let publisher = AgentPublisher::new(
        deployments,
        world.memory.clone(),
        actions_into(&world.written),
    );

    let sent = publisher
        .republish_organisation(org(), false)
        .await
        .unwrap();

    assert_eq!(sent, 1);
    let payloads = world.written.payloads();
    let last = payloads.last().unwrap();
    assert_eq!(last["enabled"], false);
    assert_eq!(last["revision"], 3);
    assert_eq!(last["policies"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_switch_skips_a_deployment_that_never_had_agent_settings() {
    let memory = Memory::default();
    let written = Written::default();
    let mut deployments = MockDeploymentRepository::new();
    deployments.expect_list_by_organisation().returning(|_| {
        let all = vec![deployment(DeploymentKind::Ferriskey)];
        Box::pin(async move { Ok(all) })
    });
    let publisher = AgentPublisher::new(deployments, memory, actions_into(&written));

    let sent = publisher.republish_organisation(org(), true).await.unwrap();

    assert_eq!(sent, 0);
    assert!(written.payloads().is_empty());
}

mod server_service_tests;
