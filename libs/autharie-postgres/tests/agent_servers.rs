use autharie_domain::{
    CoreError,
    agents::{
        AgentError, AgentPolicy, PolicyId, PolicyPhase,
        ports::AgentRepository,
        server_ports::AgentServerRepository,
        servers::{
            CustomerServer, CustomerServerUrl, DiscoveredTool, Discovery, DiscoveryPhase, ServerId,
            ServerSlug,
        },
    },
    dataplane::{
        ports::DataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneStatus, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    user::UserId,
};
use autharie_persistence::in_scratch_tx;
use autharie_postgres::{
    agents::PostgresAgentRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use uuid::Uuid;

mod support;
use support::pool;

const TEST_REGION: &str = "deployment-agents-repository-test";

async fn seed_organisation(
    tx: &autharie_persistence::SharedTx<'_>,
    label: &str,
) -> Result<(UserId, OrganisationId), CoreError> {
    let user_id = UserId(Uuid::new_v4());
    let organisation_id = OrganisationId(Uuid::new_v4());

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'bound', $3)")
        .bind(user_id.0)
        .bind(format!("{}@bound.test", user_id.0))
        .bind(user_id.0.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(|e| CoreError::DatabaseError {
            message: e.to_string(),
        })?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'active', 'free', 100, 100, 100, now(), now())",
    )
    .bind(organisation_id.0)
    .bind(label)
    .bind(format!("{label}-{}", organisation_id.0))
    .bind(user_id.0)
    .execute(&mut ***guard)
    .await
    .map_err(|e| CoreError::DatabaseError {
        message: e.to_string(),
    })?;

    Ok((user_id, organisation_id))
}

async fn seed_dataplane(tx: &autharie_persistence::SharedTx<'_>) -> Result<Uuid, CoreError> {
    let dataplanes = PostgresDataPlaneRepository::new(tx);

    let mut dataplane = autharie_domain::dataplane::entities::DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(TEST_REGION),
        Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
    );
    dataplane.status = DataPlaneStatus::Active;
    dataplanes.save(&dataplane).await?;

    Ok(dataplane.id.0)
}

fn deployment(
    organisation_id: OrganisationId,
    dataplane_id: Uuid,
    created_by: UserId,
    name: &str,
) -> Deployment {
    let at = Utc::now();

    Deployment {
        topology: Default::default(),
        id: DeploymentId(Uuid::new_v4()),
        organisation_id,
        dataplane_id: autharie_domain::dataplane::value_objects::DataPlaneId(dataplane_id),
        name: DeploymentName(name.to_string()),
        kind: DeploymentKind::Ferriskey,
        version: autharie_domain::version::Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: format!("agents-{name}"),
        environment: autharie_domain::deployments::environment::Environment::Development,
        offer: None,
        restored_from: None,
        resources: autharie_domain::dataplane::value_objects::DeploymentResources::DEFAULT,
        created_by,
        created_at: at,
        updated_at: at,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: Default::default(),
        maintenance_window: None,
        network_access: autharie_domain::deployments::network::NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    }
}

fn db_error(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

async fn seeded(
    tx: &autharie_persistence::SharedTx<'_>,
    name: &str,
) -> Result<Deployment, CoreError> {
    let (user, organisation) = seed_organisation(tx, "agents").await?;
    let dataplane = seed_dataplane(tx).await?;
    let found = deployment(organisation, dataplane, user, name);
    PostgresDeploymentRepository::new(tx)
        .insert(found.clone())
        .await?;
    Ok(found)
}

fn policy(deployment_id: DeploymentId, name: &str, enabled: bool, revision: u64) -> AgentPolicy {
    let at = Utc::now();
    AgentPolicy {
        id: PolicyId(Uuid::new_v4()),
        deployment_id,
        name: name.to_string(),
        description: Some("why".to_string()),
        enabled,
        rule: Some(json!({ "effect": "permit", "who": { "any": true } })),
        cedar: "permit(principal, action, resource);".to_string(),
        phase: enabled.then_some(PolicyPhase::Pending),
        message: None,
        desired_revision: revision,
        created_at: at,
        updated_at: at,
    }
}

fn server(deployment_id: DeploymentId, slug: &str) -> CustomerServer {
    let at = Utc::now();
    CustomerServer {
        id: ServerId(Uuid::new_v4()),
        deployment_id,
        slug: ServerSlug::try_from(slug).unwrap(),
        name: slug.to_uppercase(),
        url: CustomerServerUrl::try_from("https://tools.example.com/mcp").unwrap(),
        enabled: true,
        mcp_url: None,
        discovery: None,
        discovery_requested_at: Some(at),
        created_at: at,
        updated_at: at,
    }
}

fn discovery(at: DateTime<Utc>, names: &[&str]) -> Discovery {
    Discovery::bounded(
        DiscoveryPhase::Ready,
        names
            .iter()
            .map(|name| DiscoveredTool {
                name: name.to_string(),
                description: Some("lists".to_string()),
                read_only: true,
                destructive: false,
            })
            .collect(),
        0,
        None,
        at,
    )
}

#[tokio::test]
async fn a_server_is_stored_and_read_back_whole() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "servers-one").await?;
        let mut crm = server(dep.id, "crm");
        agents.insert_server(&crm).await?;
        crm.discovery = Some(discovery(Utc::now(), &["list_invoices"]));
        crm.name = "Billing".to_string();
        crm.enabled = false;
        let changed = agents.update_server(&crm).await?;
        let listed = agents.list_servers(dep.id).await?;
        let one = agents.get_server(dep.id, crm.id).await?;
        let other = agents
            .get_server(DeploymentId(Uuid::new_v4()), crm.id)
            .await?;
        Ok((crm, changed, listed, one, other))
    })
    .await;

    let (crm, changed, listed, one, other) = found.expect("the transaction");
    assert!(changed);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].slug.as_str(), "crm");
    assert_eq!(listed[0].name, "Billing");
    assert!(!listed[0].enabled);
    assert_eq!(listed[0].discovery, crm.discovery);
    assert_eq!(one.as_ref().map(|s| s.id), Some(crm.id));
    assert!(other.is_none());
}

#[tokio::test]
async fn a_slug_is_unique_within_a_deployment_but_not_across_them() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let first = seeded(&tx, "servers-a").await?;
        let second = seeded(&tx, "servers-b").await?;
        agents.insert_server(&server(first.id, "crm")).await?;
        agents.insert_server(&server(second.id, "crm")).await?;
        let rows = agents.list_servers(second.id).await?;
        Ok((first, rows))
    })
    .await;
    let (_, rows) = found.expect("another deployment may reuse a slug");
    assert_eq!(rows.len(), 1);

    let refused: Result<(), CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "servers-c").await?;
        agents.insert_server(&server(dep.id, "crm")).await?;
        agents.insert_server(&server(dep.id, "crm")).await
    })
    .await;

    assert!(matches!(
        refused,
        Err(CoreError::Agent(AgentError::ServerSlugTaken { ref slug })) if slug == "crm"
    ));
}

#[tokio::test]
async fn the_database_itself_refuses_a_reserved_or_malformed_slug() {
    let Some(pool) = pool().await else {
        return;
    };

    for slug in ["ferriskey", "autharie", "gateway", "Bad", "a", "1abc"] {
        let refused: Result<(), CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
            let dep = seeded(&tx, "servers-d").await?;
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO agent_servers (id, deployment_id, slug, name, url, enabled, created_at, updated_at) \
                 VALUES ($1, $2, $3, 'n', 'https://x.example/', true, now(), now())",
            )
            .bind(Uuid::new_v4())
            .bind(dep.id.0)
            .bind(slug)
            .execute(&mut ***guard)
            .await
            .map(|_| ())
            .map_err(db_error)
        })
        .await;

        assert!(refused.is_err(), "{slug}");
    }
}

#[tokio::test]
async fn removing_a_server_leaves_the_policies_of_the_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "servers-e").await?;
        let crm = server(dep.id, "crm");
        agents.insert_server(&crm).await?;
        agents
            .insert_policy(&policy(dep.id, "crm read", true, 1))
            .await?;
        let removed = agents.delete_server(dep.id, crm.id).await?;
        let again = agents.delete_server(dep.id, crm.id).await?;
        let servers = agents.list_servers(dep.id).await?;
        let policies = agents.list_policies(dep.id).await?;
        Ok((removed, again, servers, policies))
    })
    .await;

    let (removed, again, servers, policies) = found.expect("the transaction");
    assert!(removed);
    assert!(!again);
    assert!(servers.is_empty());
    assert_eq!(policies.len(), 1);
}

#[tokio::test]
async fn a_discovery_is_kept_only_when_it_is_newer_and_started_after_the_last_request() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "servers-f").await?;
        let crm = server(dep.id, "crm");
        let asked = crm.discovery_requested_at.unwrap();
        agents.insert_server(&crm).await?;

        let before_request = agents
            .record_discovery(
                dep.id,
                crm.id,
                &discovery(asked - Duration::seconds(5), &["old"]),
                None,
            )
            .await?;
        let first = agents
            .record_discovery(
                dep.id,
                crm.id,
                &discovery(asked + Duration::seconds(5), &["a"]),
                None,
            )
            .await?;
        let older = agents
            .record_discovery(
                dep.id,
                crm.id,
                &discovery(asked + Duration::seconds(2), &["b"]),
                None,
            )
            .await?;
        let newer = agents
            .record_discovery(
                dep.id,
                crm.id,
                &discovery(asked + Duration::seconds(9), &["c"]),
                None,
            )
            .await?;
        let ghost = agents
            .record_discovery(
                dep.id,
                ServerId(Uuid::new_v4()),
                &discovery(asked, &["z"]),
                None,
            )
            .await?;
        let stored = agents.get_server(dep.id, crm.id).await?.unwrap();
        Ok((before_request, first, older, newer, ghost, stored))
    })
    .await;

    let (before_request, first, older, newer, ghost, stored) = found.expect("the transaction");
    assert!(!before_request);
    assert!(first);
    assert!(!older);
    assert!(newer);
    assert!(!ghost);
    assert_eq!(stored.discovery.unwrap().tools[0].name, "c");
}

#[tokio::test]
async fn a_discovery_answers_the_request_it_echoes_whatever_the_data_planes_clock_says() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "servers-g").await?;
        let crm = server(dep.id, "crm");
        let asked = crm.discovery_requested_at.unwrap();
        agents.insert_server(&crm).await?;
        let slow_clock = discovery(asked - Duration::seconds(30), &["a"]);

        let older_request = agents
            .record_discovery(
                dep.id,
                crm.id,
                &slow_clock,
                Some(asked - Duration::seconds(60)),
            )
            .await?;
        let echoed = agents
            .record_discovery(dep.id, crm.id, &slow_clock, Some(asked))
            .await?;
        let stored = agents.get_server(dep.id, crm.id).await?.unwrap();
        Ok((older_request, echoed, stored))
    })
    .await;

    let (older_request, echoed, stored) = found.expect("the transaction");
    assert!(!older_request);
    assert!(echoed);
    assert_eq!(stored.discovery.unwrap().tools[0].name, "a");
}
