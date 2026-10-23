use std::{collections::BTreeMap, sync::Arc};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::service_tests::{
    Access, DATAPLANE, DEPLOYMENT, Memory, ORGANISATION, Users, Written, actions_into, caller, dep,
    deployments_of, ingest_on, org, organisation, outcome,
};
use crate::{
    CoreError,
    action::ports::MockActionRepository,
    agents::{
        AgentError, AgentSettingsInput,
        ports::{AgentOutcomeService, AgentRepository},
        registry::{
            AccessState, AgentClientId, AutonomousInput, ClientKind, ClientSource, CredentialFetch,
            ReportedClient, ReportedCredential, ReportedRegistry, SealingJwk,
        },
        registry_ports::{AgentActivityIndex, AgentRegistryService},
        registry_service::AgentRegistryServiceImpl,
    },
    audit::ports::MockAuditRepository,
    deployments::{DeploymentId, DeploymentKind, ports::MockDeploymentRepository},
    organisation::{OrganisationId, ports::MockOrganisationRepository},
};

type Service = AgentRegistryServiceImpl<
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

fn world(kind: DeploymentKind, manage: bool) -> World {
    let memory = Memory::default();
    let written = Written::default();

    let mut organisations = MockOrganisationRepository::new();
    organisations.expect_find_by_id().returning(move |_| {
        let found = organisation(true);
        Box::pin(async move { Ok(Some(found)) })
    });

    let mut audit = MockAuditRepository::new();
    let held = Arc::clone(&written.audit);
    audit.expect_append().returning(move |entry| {
        held.lock().unwrap().push(entry);
        Box::pin(async { Ok(()) })
    });

    World {
        service: AgentRegistryServiceImpl::new(
            deployments_of(kind),
            organisations,
            memory.clone(),
            actions_into(&written),
            audit,
            Users,
            Access { manage },
            Some("example.test".to_string()),
        ),
        memory,
        written,
    }
}

async fn enabled_world() -> World {
    let world = world(DeploymentKind::Ferriskey, true);
    world
        .memory
        .save_settings(
            dep(),
            &AgentSettingsInput::parse(true, Some("acme")).unwrap(),
            Utc::now(),
        )
        .await
        .unwrap();
    world
}

async fn seeded() -> Memory {
    let memory = Memory::default();
    memory
        .save_settings(
            dep(),
            &AgentSettingsInput::parse(true, Some("acme")).unwrap(),
            Utc::now(),
        )
        .await
        .unwrap();
    memory
}

fn client(value: &str) -> AgentClientId {
    AgentClientId::try_from(value).unwrap()
}

fn jwk() -> SealingJwk {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let mut modulus = vec![0xffu8; 256];
    modulus[255] = 0xfd;
    SealingJwk {
        kty: "RSA".to_string(),
        n: URL_SAFE_NO_PAD.encode(modulus),
        e: "AQAB".to_string(),
        alg: Some("RSA-OAEP-256".to_string()),
        d: None,
    }
}

fn input() -> AutonomousInput {
    AutonomousInput::parse("n8n workflow", None, jwk()).unwrap()
}

fn last(world: &World) -> Value {
    world.written.payloads().last().cloned().expect("an action")
}

fn registered(id: &str, source: ClientSource) -> ReportedClient {
    ReportedClient {
        client_id: client(id),
        name: None,
        source,
        kind: ClientKind::Delegated,
        enabled: true,
        created_at: None,
    }
}

struct Seen(BTreeMap<String, DateTime<Utc>>);

impl AgentActivityIndex for Seen {
    async fn last_seen(
        &self,
        _: OrganisationId,
        _: DeploymentId,
        _: DateTime<Utc>,
    ) -> Result<BTreeMap<String, DateTime<Utc>>, CoreError> {
        Ok(self.0.clone())
    }
}

struct Failing;

impl AgentActivityIndex for Failing {
    async fn last_seen(
        &self,
        _: OrganisationId,
        _: DeploymentId,
        _: DateTime<Utc>,
    ) -> Result<BTreeMap<String, DateTime<Utc>>, CoreError> {
        Err(CoreError::InternalError("quickwit is down".to_string()))
    }
}

fn report_sealed(
    world: &World,
    revision: u64,
    request_id: Uuid,
    client_id: &str,
    sealed: &str,
) -> crate::agents::AgentsOutcome {
    let _ = world;
    outcome(revision, "Ready", vec![]).with_registry(
        ReportedRegistry::default(),
        vec![ReportedCredential::parse(request_id, client_id, sealed).unwrap()],
    )
}

#[tokio::test]
async fn turning_approval_mode_on_is_sent_to_the_data_plane() {
    let world = world(DeploymentKind::Ferriskey, true);

    world
        .service
        .set_approval_mode(caller(), org(), dep(), true)
        .await
        .expect("saved");

    assert_eq!(last(&world)["access"]["approval_required"], true);
    assert_eq!(world.written.audited().len(), 1);
}

#[tokio::test]
async fn approving_and_blocking_an_agent_reach_the_action_and_unblocking_takes_it_back() {
    let world = world(DeploymentKind::Ferriskey, true);

    world
        .service
        .approve_agent(caller(), org(), dep(), client("a"))
        .await
        .unwrap();
    assert_eq!(last(&world)["access"]["approved"], json!(["a"]));

    world
        .service
        .block_agent(caller(), org(), dep(), client("a"))
        .await
        .unwrap();
    assert_eq!(last(&world)["access"]["blocked"], json!(["a"]));
    assert_eq!(last(&world)["access"]["approved"], json!(["a"]));

    world
        .service
        .unblock_agent(caller(), org(), dep(), client("a"))
        .await
        .unwrap();
    assert_eq!(last(&world)["access"]["blocked"], json!([]));
}

#[tokio::test]
async fn every_registry_change_takes_a_new_revision() {
    let world = world(DeploymentKind::Ferriskey, true);

    world
        .service
        .approve_agent(caller(), org(), dep(), client("a"))
        .await
        .unwrap();
    world
        .service
        .block_agent(caller(), org(), dep(), client("a"))
        .await
        .unwrap();

    let revisions: Vec<_> = world
        .written
        .payloads()
        .iter()
        .map(|payload| payload["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, vec![1, 2]);
}

#[tokio::test]
async fn deleting_an_agent_asks_the_data_plane_to_delete_it_and_forgets_its_access() {
    let world = enabled_world().await;
    world
        .service
        .approve_agent(caller(), org(), dep(), client("old"))
        .await
        .unwrap();

    world
        .service
        .delete_agent(caller(), org(), dep(), client("old"))
        .await
        .unwrap();

    let payload = last(&world);
    assert_eq!(payload["deleted_agents"], json!(["old"]));
    assert_eq!(payload["access"]["approved"], json!([]));
}

#[tokio::test]
async fn deleting_an_autonomous_agent_removes_it_from_the_desired_state() {
    let world = enabled_world().await;
    world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();
    let created = last(&world)["autonomous"][0]["client_id"]
        .as_str()
        .unwrap()
        .to_string();

    world
        .service
        .delete_agent(caller(), org(), dep(), client(&created))
        .await
        .unwrap();

    let payload = last(&world);
    assert_eq!(payload["autonomous"], json!([]));
    assert_eq!(payload["deleted_agents"], json!([created]));
}

#[tokio::test]
async fn an_autonomous_agent_needs_agents_enabled_on_the_deployment() {
    let world = world(DeploymentKind::Ferriskey, true);

    let error = world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .expect_err("refused");

    assert!(matches!(error, CoreError::Agent(AgentError::NotEnabled)));
    assert!(world.written.payloads().is_empty());
}

#[tokio::test]
async fn spec_ar_5_creating_an_autonomous_agent_sends_the_request_and_the_sealing_key() {
    let world = enabled_world().await;

    let request_id = world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .expect("accepted");

    let entry = &last(&world)["autonomous"][0];
    assert!(
        entry["client_id"]
            .as_str()
            .unwrap()
            .starts_with("agent-n8n-workflow-")
    );
    assert_eq!(entry["name"], "n8n workflow");
    assert_eq!(entry["request_id"], json!(request_id));
    assert_eq!(entry["sealing_key"]["alg"], "RSA-OAEP-256");
    assert_eq!(entry["sealing_key"]["kty"], "RSA");
}

#[tokio::test]
async fn an_autonomous_agent_the_owner_creates_is_approved() {
    let world = enabled_world().await;

    world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .expect("accepted");

    let payload = last(&world);
    assert_eq!(
        payload["access"]["approved"],
        json!([payload["autonomous"][0]["client_id"]])
    );
}

#[tokio::test]
async fn spec_ar_5_the_sealed_secret_is_handed_out_once_then_gone() {
    let world = enabled_world().await;
    let request_id = world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();
    let client_id = last(&world)["autonomous"][0]["client_id"]
        .as_str()
        .unwrap()
        .to_string();

    let pending = world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .unwrap();
    assert_eq!(pending, CredentialFetch::Pending);

    let ingest = ingest_on(&world.memory, DATAPLANE);
    ingest
        .report_agents_outcome(
            caller(),
            dep(),
            report_sealed(&world, 1, request_id, &client_id, "c2VhbGVk"),
        )
        .await
        .unwrap();

    let delivered = world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .unwrap();
    let CredentialFetch::Delivered(credential) = delivered else {
        panic!("expected the credential");
    };
    assert_eq!(credential.sealed, "c2VhbGVk");
    assert_eq!(credential.client_id.as_str(), client_id);
    assert_eq!(credential.audience, "https://reported.example/mcp");
    assert_eq!(
        credential.token_url,
        "https://reported.example/api/realms/acme/protocol/openid-connect/token"
    );

    let second = world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .expect_err("gone");
    assert!(matches!(
        second,
        CoreError::Agent(AgentError::CredentialGone { .. })
    ));
    world.memory.read(|state| {
        assert!(state.requests.iter().all(|stored| stored.sealed.is_none()));
    });
}

#[tokio::test]
async fn after_delivery_the_desired_state_no_longer_carries_the_request() {
    let world = enabled_world().await;
    let request_id = world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();
    let client_id = last(&world)["autonomous"][0]["client_id"]
        .as_str()
        .unwrap()
        .to_string();
    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            report_sealed(&world, 1, request_id, &client_id, "c2VhbGVk"),
        )
        .await
        .unwrap();

    world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .unwrap();

    let entry = &last(&world)["autonomous"][0];
    assert!(entry.get("request_id").is_none());
    assert!(entry.get("sealing_key").is_none());
}

#[tokio::test]
async fn a_credential_reported_for_a_request_nobody_is_waiting_for_is_not_stored() {
    let world = enabled_world().await;

    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            report_sealed(&world, 0, Uuid::from_u128(77), "agent-x-abc123", "c2VhbGVk"),
        )
        .await
        .unwrap();

    world
        .memory
        .read(|state| assert!(state.requests.is_empty()));
}

#[tokio::test]
async fn a_delivered_credential_is_not_stored_again_when_the_data_plane_reports_it_again() {
    let world = enabled_world().await;
    let request_id = world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();
    let client_id = last(&world)["autonomous"][0]["client_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ingest = ingest_on(&world.memory, DATAPLANE);
    ingest
        .report_agents_outcome(
            caller(),
            dep(),
            report_sealed(&world, 1, request_id, &client_id, "c2VhbGVk"),
        )
        .await
        .unwrap();
    world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .unwrap();

    ingest
        .report_agents_outcome(
            caller(),
            dep(),
            report_sealed(&world, 2, request_id, &client_id, "c2VhbGVk"),
        )
        .await
        .unwrap();

    let again = world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .expect_err("still gone");
    assert!(matches!(
        again,
        CoreError::Agent(AgentError::CredentialGone { .. })
    ));
}

#[tokio::test]
async fn spec_ar_6_a_credential_not_fetched_within_fifteen_minutes_is_deleted() {
    let world = enabled_world().await;
    let request_id = world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();
    world.memory.0.lock().unwrap().requests[0]
        .request
        .expires_at = Utc::now() - Duration::seconds(1);

    let error = world
        .service
        .fetch_agent_credential(caller(), org(), dep(), request_id)
        .await
        .expect_err("expired");

    assert!(matches!(
        error,
        CoreError::Agent(AgentError::CredentialNotFound { .. })
    ));
    world
        .memory
        .read(|state| assert!(state.requests.is_empty()));
}

#[tokio::test]
async fn a_request_lives_fifteen_minutes() {
    let world = enabled_world().await;
    let before = Utc::now();

    world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();

    let expires = world
        .memory
        .read(|state| state.requests[0].request.expires_at);
    assert!(expires >= before + Duration::minutes(15));
    assert!(expires <= Utc::now() + Duration::minutes(15));
}

#[tokio::test]
async fn an_unknown_credential_request_is_not_found() {
    let world = enabled_world().await;

    let error = world
        .service
        .fetch_agent_credential(caller(), org(), dep(), Uuid::from_u128(5))
        .await
        .expect_err("unknown");

    assert!(matches!(
        error,
        CoreError::Agent(AgentError::CredentialNotFound { .. })
    ));
}

#[tokio::test]
async fn rotating_a_known_autonomous_agent_sends_a_rotation_request() {
    let world = enabled_world().await;
    world
        .service
        .create_autonomous_agent(caller(), org(), dep(), input())
        .await
        .unwrap();
    let client_id = last(&world)["autonomous"][0]["client_id"]
        .as_str()
        .unwrap()
        .to_string();
    world.memory.0.lock().unwrap().requests.clear();

    let request_id = world
        .service
        .rotate_autonomous_secret(
            caller(),
            org(),
            dep(),
            client(&client_id),
            input().sealing_key,
        )
        .await
        .unwrap();

    let entry = &last(&world)["autonomous"][0];
    assert_eq!(entry["rotate_request_id"], json!(request_id));
    assert!(entry.get("request_id").is_none());
}

#[tokio::test]
async fn rotating_an_agent_autharie_did_not_create_is_refused() {
    let world = enabled_world().await;

    let error = world
        .service
        .rotate_autonomous_secret(caller(), org(), dep(), client("dcr"), input().sealing_key)
        .await
        .expect_err("refused");

    assert!(matches!(
        error,
        CoreError::Agent(AgentError::AutonomousAgentNotFound { .. })
    ));
}

#[tokio::test]
async fn a_viewer_cannot_change_access_nor_fetch_a_credential() {
    let world = world(DeploymentKind::Ferriskey, false);

    assert!(
        world
            .service
            .approve_agent(caller(), org(), dep(), client("a"))
            .await
            .is_err()
    );
    assert!(
        world
            .service
            .fetch_agent_credential(caller(), org(), dep(), Uuid::nil())
            .await
            .is_err()
    );
    assert!(world.written.payloads().is_empty());
}

#[tokio::test]
async fn a_viewer_can_read_the_registry() {
    let world = world(DeploymentKind::Ferriskey, false);

    let view = world
        .service
        .list_agent_clients(caller(), org(), dep(), Seen(BTreeMap::new()))
        .await
        .expect("allowed");

    assert!(view.data.is_empty());
}

#[tokio::test]
async fn the_registry_is_for_ferriskey_deployments_only() {
    let world = world(DeploymentKind::Keycloak, true);

    let error = world
        .service
        .list_agent_clients(caller(), org(), dep(), Seen(BTreeMap::new()))
        .await
        .expect_err("refused");

    assert!(matches!(
        error,
        CoreError::Agent(AgentError::UnsupportedKind)
    ));
}

#[tokio::test]
async fn spec_ar_7_last_activity_is_the_newest_decision_of_the_agent() {
    let world = world(DeploymentKind::Ferriskey, true);
    world.memory.0.lock().unwrap().registry = ReportedRegistry {
        clients: vec![registered("dcr", ClientSource::Dynamic)],
        truncated: false,
    };
    let newest = Utc.timestamp_opt(1_800_000_000, 0).unwrap();

    let view = world
        .service
        .list_agent_clients(
            caller(),
            org(),
            dep(),
            Seen(BTreeMap::from([("dcr".to_string(), newest)])),
        )
        .await
        .unwrap();

    assert_eq!(view.data[0].last_seen_at, Some(newest));
}

#[tokio::test]
async fn the_registry_still_lists_when_the_activity_index_is_down() {
    let world = world(DeploymentKind::Ferriskey, true);
    world.memory.0.lock().unwrap().registry = ReportedRegistry {
        clients: vec![registered("dcr", ClientSource::Dynamic)],
        truncated: false,
    };

    let view = world
        .service
        .list_agent_clients(caller(), org(), dep(), Failing)
        .await
        .unwrap();

    assert_eq!(view.data.len(), 1);
    assert_eq!(view.data[0].last_seen_at, None);
}

#[tokio::test]
async fn spec_ar_2_a_reported_agent_is_pending_in_approval_mode_and_approved_after_approval() {
    let world = world(DeploymentKind::Ferriskey, true);
    world.memory.0.lock().unwrap().registry = ReportedRegistry {
        clients: vec![registered("dcr", ClientSource::Dynamic)],
        truncated: false,
    };
    world
        .service
        .set_approval_mode(caller(), org(), dep(), true)
        .await
        .unwrap();

    let before = world
        .service
        .list_agent_clients(caller(), org(), dep(), Seen(BTreeMap::new()))
        .await
        .unwrap();
    world
        .service
        .approve_agent(caller(), org(), dep(), client("dcr"))
        .await
        .unwrap();
    let after = world
        .service
        .list_agent_clients(caller(), org(), dep(), Seen(BTreeMap::new()))
        .await
        .unwrap();

    assert_eq!(before.data[0].access, AccessState::Pending);
    assert_eq!(after.data[0].access, AccessState::Approved);
}

#[tokio::test]
async fn the_reported_registry_is_bounded_to_500_newest_first_with_a_truncated_flag() {
    let memory = seeded().await;
    let clients: Vec<_> = (0..600)
        .map(|n| registered(&format!("client-{n}"), ClientSource::Dynamic))
        .collect();
    let report = outcome(0, "Ready", vec![]).with_registry(
        ReportedRegistry {
            clients,
            truncated: false,
        },
        vec![],
    );

    ingest_on(&memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), report)
        .await
        .unwrap();

    memory.read(|state| {
        assert_eq!(state.registry.clients.len(), 500);
        assert!(state.registry.truncated);
        assert_eq!(state.registry.clients[0].client_id.as_str(), "client-0");
    });
}

#[tokio::test]
async fn a_deletion_is_forgotten_once_the_data_plane_no_longer_reports_the_client() {
    let memory = seeded().await;
    {
        let mut state = memory.0.lock().unwrap();
        state.access.deleted.insert("gone".to_string());
        state.access.deleted.insert("still-there".to_string());
    }
    let report = outcome(0, "Ready", vec![]).with_registry(
        ReportedRegistry {
            clients: vec![registered("still-there", ClientSource::Dynamic)],
            truncated: false,
        },
        vec![],
    );

    ingest_on(&memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), report)
        .await
        .unwrap();

    memory.read(|state| {
        assert_eq!(
            state.access.deleted.iter().collect::<Vec<_>>(),
            vec!["still-there"]
        );
    });
}

#[tokio::test]
async fn a_truncated_registry_never_forgets_a_deletion() {
    let memory = seeded().await;
    memory
        .0
        .lock()
        .unwrap()
        .access
        .deleted
        .insert("gone".to_string());
    let report = outcome(0, "Ready", vec![]).with_registry(
        ReportedRegistry {
            clients: vec![],
            truncated: true,
        },
        vec![],
    );

    ingest_on(&memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), report)
        .await
        .unwrap();

    memory.read(|state| assert!(state.access.deleted.contains("gone")));
}

#[test]
fn the_identifiers_used_by_the_world_are_the_ones_the_tests_expect() {
    assert_eq!(org(), OrganisationId(ORGANISATION));
    assert_eq!(dep(), DeploymentId(DEPLOYMENT));
}
