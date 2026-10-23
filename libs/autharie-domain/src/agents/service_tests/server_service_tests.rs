use chrono::Duration;

use super::*;
use crate::agents::{
    AgentError,
    server_ports::AgentServerService,
    servers::{DiscoveredTool, DiscoveryPhase, MAX_SERVERS, ServerInput, ServerOutcome},
};

fn crm() -> ServerInput {
    ServerInput::register("crm", "CRM", "https://tools.example.com/mcp", None).unwrap()
}

fn registered(slug: &str) -> ServerInput {
    ServerInput::register(slug, slug, "https://tools.example.com/mcp", None).unwrap()
}

async fn register(world: &World, input: ServerInput) -> CustomerServer {
    world
        .service
        .register_agent_server(caller(), org(), dep(), input)
        .await
        .expect("registered")
}

fn discovered(id: ServerId, at: DateTime<Utc>, names: &[&str]) -> ServerOutcome {
    ServerOutcome {
        id,
        discovery: Some(Discovery::bounded(
            DiscoveryPhase::Ready,
            names
                .iter()
                .map(|name| DiscoveredTool {
                    name: name.to_string(),
                    description: None,
                    read_only: true,
                    destructive: false,
                })
                .collect(),
            0,
            None,
            at,
        )),
        answers: None,
    }
}

fn with_servers(world: &World, revision: u64, servers: Vec<ServerOutcome>) -> AgentsOutcome {
    let _ = world;
    outcome(revision, "Ready", vec![]).with_servers(servers)
}

#[tokio::test]
async fn a_registered_server_bumps_the_revision_and_travels_in_the_action() {
    let world = ferriskey(true);
    enable(&world).await;

    let server = register(&world, crm()).await;

    let payloads = world.written.payloads();
    let last = payloads.last().unwrap();
    assert_eq!(last["revision"], 2);
    let sent = &last["servers"][0];
    assert_eq!(sent["id"], json!(server.id.0));
    assert_eq!(sent["slug"], "crm");
    assert_eq!(sent["url"], "https://tools.example.com/mcp");
    assert_eq!(sent["enabled"], true);
    assert!(sent["discovery_requested_at"].is_string());
    assert!(
        world
            .written
            .audited()
            .contains(&AGENT_SERVER_CREATED_AUDIT.to_string())
    );
}

#[tokio::test]
async fn a_server_slug_can_only_be_used_once_per_deployment() {
    let world = ferriskey(true);
    register(&world, crm()).await;

    let error = world
        .service
        .register_agent_server(caller(), org(), dep(), crm())
        .await
        .expect_err("refused");

    assert!(matches!(
        error,
        CoreError::Agent(AgentError::ServerSlugTaken { .. })
    ));
}

#[tokio::test]
async fn a_deployment_holds_at_most_ten_servers() {
    let world = ferriskey(true);
    for index in 0..MAX_SERVERS {
        register(&world, registered(&format!("srv-{index}"))).await;
    }

    let error = world
        .service
        .register_agent_server(caller(), org(), dep(), registered("one-more"))
        .await
        .expect_err("refused");

    assert!(matches!(
        error,
        CoreError::Agent(AgentError::TooManyServers { max: 10 })
    ));
    assert_eq!(world.memory.read(|state| state.servers.len()), 10);
}

#[tokio::test]
async fn servers_are_for_ferriskey_deployments_and_for_those_who_may_manage() {
    let other = world(DeploymentKind::Keycloak, true, true, None);
    let read_only = world(DeploymentKind::Ferriskey, true, false, None);

    assert!(matches!(
        other
            .service
            .register_agent_server(caller(), org(), dep(), crm())
            .await,
        Err(CoreError::Agent(AgentError::UnsupportedKind))
    ));
    assert!(matches!(
        read_only
            .service
            .register_agent_server(caller(), org(), dep(), crm())
            .await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(read_only.written.payloads().is_empty());
}

#[tokio::test]
async fn listing_gives_each_server_its_own_mcp_url() {
    let world = world(DeploymentKind::Ferriskey, true, true, Some("example.test"));
    register(&world, crm()).await;

    let listed = world
        .service
        .list_agent_servers(caller(), org(), dep())
        .await
        .unwrap();

    assert_eq!(
        listed[0].mcp_url.as_deref(),
        Some("https://auth.example.test/mcp/crm")
    );
}

#[tokio::test]
async fn moving_a_server_forgets_its_tools_and_asks_for_a_new_discovery() {
    let world = ferriskey(true);
    let server = register(&world, crm()).await;
    let asked = server.discovery_requested_at.unwrap();
    world
        .memory
        .record_discovery(
            dep(),
            server.id,
            &discovered(server.id, asked + Duration::seconds(5), &["list_invoices"])
                .discovery
                .unwrap(),
            None,
        )
        .await
        .unwrap();

    let moved = world
        .service
        .update_agent_server(
            caller(),
            org(),
            dep(),
            server.id,
            ServerInput::change("CRM", "https://other.example.com/mcp", true).unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(moved.discovery, None);
    assert!(moved.discovery_requested_at.unwrap() > asked);
    assert_eq!(moved.slug, server.slug);
    assert_eq!(
        world.written.payloads().last().unwrap()["servers"][0]["url"],
        "https://other.example.com/mcp"
    );
}

#[tokio::test]
async fn renaming_or_switching_off_a_server_keeps_what_was_discovered() {
    let world = ferriskey(true);
    let server = register(&world, crm()).await;
    let at = server.discovery_requested_at.unwrap() + Duration::seconds(5);
    world
        .memory
        .record_discovery(
            dep(),
            server.id,
            &discovered(server.id, at, &["list_invoices"])
                .discovery
                .unwrap(),
            None,
        )
        .await
        .unwrap();

    let off = world
        .service
        .update_agent_server(
            caller(),
            org(),
            dep(),
            server.id,
            ServerInput::change("Billing CRM", "https://tools.example.com/mcp", false).unwrap(),
        )
        .await
        .unwrap();

    assert!(!off.enabled);
    assert_eq!(off.discovery.unwrap().tools.len(), 1);
    assert_eq!(
        world.written.payloads().last().unwrap()["servers"][0]["enabled"],
        false
    );
}

#[tokio::test]
async fn asking_for_a_discovery_bumps_the_revision_and_stamps_the_request() {
    let world = ferriskey(true);
    let server = register(&world, crm()).await;
    let before = world.written.payloads().len();

    world
        .service
        .discover_agent_server(caller(), org(), dep(), server.id)
        .await
        .unwrap();

    let payloads = world.written.payloads();
    assert_eq!(payloads.len(), before + 1);
    let asked = payloads.last().unwrap()["servers"][0]["discovery_requested_at"]
        .as_str()
        .unwrap()
        .parse::<DateTime<Utc>>()
        .unwrap();
    assert!(asked > server.discovery_requested_at.unwrap());
    assert!(
        payloads.last().unwrap()["revision"].as_u64() > payloads[before - 1]["revision"].as_u64()
    );
}

#[tokio::test]
async fn removing_a_server_leaves_the_policies_that_name_it() {
    let world = ferriskey(true);
    enable(&world).await;
    let server = register(&world, crm()).await;
    let policy = add(&world, "crm read", true).await;

    world
        .service
        .remove_agent_server(caller(), org(), dep(), server.id)
        .await
        .unwrap();

    let payloads = world.written.payloads();
    let last = payloads.last().unwrap();
    assert_eq!(last["servers"], json!([]));
    assert_eq!(last["policies"][0]["id"], json!(policy.id.0));
    assert!(world.memory.read(|state| state.servers.is_empty()));
    assert_eq!(world.memory.read(|state| state.policies.len()), 1);
}

#[tokio::test]
async fn a_server_that_does_not_exist_is_not_found() {
    let world = ferriskey(true);
    let ghost = ServerId(Uuid::from_u128(99));

    let removed = world
        .service
        .remove_agent_server(caller(), org(), dep(), ghost)
        .await;
    let asked = world
        .service
        .discover_agent_server(caller(), org(), dep(), ghost)
        .await;

    for error in [removed.unwrap_err(), asked.unwrap_err()] {
        assert!(matches!(
            error,
            CoreError::Agent(AgentError::ServerNotFound { .. })
        ));
    }
}

#[tokio::test]
async fn what_the_data_plane_discovered_is_stored_with_the_report() {
    let world = ferriskey(true);
    enable(&world).await;
    let server = register(&world, crm()).await;
    let revision = world
        .memory
        .read(|state| state.settings.as_ref().unwrap().revision);
    let at = server.discovery_requested_at.unwrap() + Duration::seconds(2);

    let recorded = ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            with_servers(
                &world,
                revision,
                vec![discovered(server.id, at, &["list_invoices", "bad name"])],
            ),
        )
        .await
        .unwrap();

    assert!(recorded);
    let stored = world
        .service
        .list_agent_servers(caller(), org(), dep())
        .await
        .unwrap()
        .remove(0);
    let discovery = stored.discovery.expect("discovered");
    assert_eq!(discovery.phase, DiscoveryPhase::Ready);
    assert_eq!(discovery.tools.len(), 1);
    assert_eq!(discovery.tools[0].name, "list_invoices");
    assert_eq!(discovery.dropped, 1);
}

#[tokio::test]
async fn a_discovery_started_before_the_server_last_changed_is_ignored() {
    let world = ferriskey(true);
    enable(&world).await;
    let server = register(&world, crm()).await;
    let revision = world
        .memory
        .read(|state| state.settings.as_ref().unwrap().revision);
    let stale = server.discovery_requested_at.unwrap() - Duration::seconds(30);

    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            with_servers(
                &world,
                revision,
                vec![discovered(server.id, stale, &["old"])],
            ),
        )
        .await
        .unwrap();

    assert_eq!(
        world
            .memory
            .read(|state| state.servers[0].discovery.clone()),
        None
    );
}

#[tokio::test]
async fn a_report_about_a_server_that_was_removed_changes_nothing() {
    let world = ferriskey(true);
    enable(&world).await;
    let revision = world
        .memory
        .read(|state| state.settings.as_ref().unwrap().revision);

    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(
            caller(),
            dep(),
            with_servers(
                &world,
                revision,
                vec![discovered(
                    ServerId(Uuid::from_u128(77)),
                    Utc::now(),
                    &["x"],
                )],
            ),
        )
        .await
        .unwrap();

    assert!(world.memory.read(|state| state.servers.is_empty()));
}

#[tokio::test]
async fn a_discovery_that_echoes_the_request_it_answers_is_kept_even_if_the_data_plane_clock_runs_behind()
 {
    let world = ferriskey(true);
    enable(&world).await;
    let server = register(&world, crm()).await;
    let revision = world
        .memory
        .read(|state| state.settings.as_ref().unwrap().revision);
    let asked = server.discovery_requested_at.unwrap();
    let mut late = discovered(server.id, asked - Duration::seconds(20), &["list_invoices"]);
    late.answers = Some(asked.duration_trunc(Duration::milliseconds(1)).unwrap());

    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), with_servers(&world, revision, vec![late]))
        .await
        .unwrap();

    let stored = world
        .memory
        .read(|state| state.servers[0].discovery.clone());
    assert_eq!(stored.unwrap().tools[0].name, "list_invoices");
}

#[tokio::test]
async fn a_discovery_that_answers_an_older_request_is_dropped_whatever_its_clock_says() {
    let world = ferriskey(true);
    enable(&world).await;
    let server = register(&world, crm()).await;
    let revision = world
        .memory
        .read(|state| state.settings.as_ref().unwrap().revision);
    let asked = server.discovery_requested_at.unwrap();
    let mut old = discovered(server.id, asked + Duration::seconds(20), &["old"]);
    old.answers = Some(asked - Duration::seconds(60));

    ingest_on(&world.memory, DATAPLANE)
        .report_agents_outcome(caller(), dep(), with_servers(&world, revision, vec![old]))
        .await
        .unwrap();

    assert_eq!(
        world
            .memory
            .read(|state| state.servers[0].discovery.clone()),
        None
    );
}
