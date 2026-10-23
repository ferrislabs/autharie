use autharie_domain::{
    CoreError,
    agents::{
        registry::{
            AccessLists, AgentClientId, AutonomousAgent, ClientKind, ClientSource,
            CredentialRequest, CredentialState, ReportedClient, ReportedRegistry, RequestKind,
            SealingJwk, SealingKey,
        },
        registry_ports::AgentRegistryRepository,
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
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use uuid::Uuid;

mod support;
use support::pool;

const TEST_REGION: &str = "agent-registry-repository-test";

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

fn key() -> SealingKey {
    let mut modulus = vec![0xffu8; 256];
    modulus[255] = 0xfd;
    SealingKey::try_from(SealingJwk {
        kty: "RSA".to_string(),
        n: URL_SAFE_NO_PAD.encode(modulus),
        e: "AQAB".to_string(),
        alg: Some("RSA-OAEP-256".to_string()),
        d: None,
    })
    .unwrap()
}

fn id(value: &str) -> AgentClientId {
    AgentClientId::try_from(value).unwrap()
}

fn request(client: &str, expires_in: Duration) -> CredentialRequest {
    CredentialRequest {
        request_id: Uuid::new_v4(),
        client_id: id(client),
        kind: RequestKind::Create,
        sealing_key: key(),
        expires_at: Utc::now() + expires_in,
    }
}

fn reported(client: &str, source: ClientSource) -> ReportedClient {
    ReportedClient {
        client_id: id(client),
        name: Some(format!("name of {client}")),
        source,
        kind: ClientKind::Delegated,
        enabled: true,
        created_at: Some(Utc::now()),
    }
}

async fn autonomous(
    repository: &PostgresAgentRepository<'_>,
    deployment: DeploymentId,
    client: &str,
) -> Result<(), CoreError> {
    repository
        .insert_autonomous_agent(
            deployment,
            &AutonomousAgent {
                client_id: id(client),
                name: "n8n".to_string(),
                description: Some("nightly".to_string()),
                created_at: Utc::now(),
            },
        )
        .await
}

#[tokio::test]
async fn access_lists_default_to_open_and_empty_then_round_trip() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "access").await?;
        let empty = repository.access(dep.id).await?;
        let mut lists = AccessLists {
            approval_required: true,
            ..AccessLists::default()
        };
        lists.approve(&id("a"));
        lists.block(&id("b"));
        lists.forget(&id("c"));
        repository.save_access(dep.id, &lists, Utc::now()).await?;
        let read = repository.access(dep.id).await?;
        Ok((empty, lists, read))
    })
    .await;

    let (empty, saved, read) = found.expect("the transaction");
    assert_eq!(empty, AccessLists::default());
    assert_eq!(read, saved);
}

#[tokio::test]
async fn replacing_the_registry_keeps_the_reported_order_and_drops_what_is_gone() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "registry").await?;
        let first = ReportedRegistry {
            clients: vec![
                reported("newest", ClientSource::Dynamic),
                reported("middle", ClientSource::MetadataDocument),
                reported("oldest", ClientSource::Dynamic),
            ],
            truncated: false,
        };
        repository
            .replace_registry(dep.id, &first, Utc::now())
            .await?;
        let second = ReportedRegistry {
            clients: vec![
                reported("fresh", ClientSource::Dynamic),
                reported("oldest", ClientSource::Dynamic),
            ],
            truncated: true,
        };
        repository
            .replace_registry(dep.id, &second, Utc::now())
            .await?;
        let read = repository.registry(dep.id).await?;
        let none = repository.registry(DeploymentId(Uuid::new_v4())).await?;
        Ok((read, none))
    })
    .await;

    let (read, none) = found.expect("the transaction");
    let ids: Vec<_> = read.clients.iter().map(|c| c.client_id.as_str()).collect();
    assert_eq!(ids, vec!["fresh", "oldest"]);
    assert!(read.truncated);
    assert_eq!(read.clients[0].name.as_deref(), Some("name of fresh"));
    assert_eq!(none, ReportedRegistry::default());
}

#[tokio::test]
async fn replacing_the_registry_leaves_the_access_lists_alone() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "alone").await?;
        let mut lists = AccessLists::default();
        lists.approve(&id("a"));
        repository.save_access(dep.id, &lists, Utc::now()).await?;
        repository
            .replace_registry(dep.id, &ReportedRegistry::default(), Utc::now())
            .await?;
        repository.access(dep.id).await
    })
    .await;

    assert!(found.expect("the transaction").approved.contains("a"));
}

#[tokio::test]
async fn spec_ar_5_a_sealed_secret_is_taken_once_and_leaves_no_secret_behind() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "credential").await?;
        autonomous(&repository, dep.id, "agent-n8n-abc123").await?;
        let asked = request("agent-n8n-abc123", Duration::minutes(15));
        let now = Utc::now();
        repository.insert_request(dep.id, &asked, now).await?;

        let open = repository.open_requests(dep.id, now).await?;
        let pending = repository
            .credential_state(dep.id, asked.request_id, now)
            .await?;
        let stored = repository
            .store_sealed(
                dep.id,
                asked.request_id,
                &id("agent-n8n-abc123"),
                "c2VhbGVk",
                now,
            )
            .await?;
        let stored_twice = repository
            .store_sealed(
                dep.id,
                asked.request_id,
                &id("agent-n8n-abc123"),
                "b3RoZXI",
                now,
            )
            .await?;
        let open_after = repository.open_requests(dep.id, now).await?;
        let ready = repository
            .credential_state(dep.id, asked.request_id, now)
            .await?;
        let first = repository
            .take_sealed(dep.id, asked.request_id, now)
            .await?;
        let second = repository
            .take_sealed(dep.id, asked.request_id, now)
            .await?;
        let delivered = repository
            .credential_state(dep.id, asked.request_id, now)
            .await?;
        let late = repository
            .store_sealed(
                dep.id,
                asked.request_id,
                &id("agent-n8n-abc123"),
                "c2VhbGVk",
                now,
            )
            .await?;

        let mut guard = tx.lock().await;
        let remaining: Option<String> =
            sqlx::query_scalar("SELECT sealed FROM agent_credentials WHERE request_id = $1")
                .bind(asked.request_id)
                .fetch_one(&mut ***guard)
                .await
                .map_err(db_error)?;
        Ok((
            open,
            pending,
            stored,
            stored_twice,
            open_after,
            ready,
            first,
            second,
            delivered,
            late,
            remaining,
        ))
    })
    .await;

    let (
        open,
        pending,
        stored,
        stored_twice,
        open_after,
        ready,
        first,
        second,
        delivered,
        late,
        remaining,
    ) = found.expect("the transaction");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].sealing_key, key());
    assert_eq!(pending, CredentialState::Pending);
    assert!(stored);
    assert!(!stored_twice);
    assert!(open_after.is_empty());
    assert_eq!(ready, CredentialState::Ready);
    assert_eq!(first.expect("taken").sealed, "c2VhbGVk");
    assert!(second.is_none());
    assert_eq!(delivered, CredentialState::Delivered);
    assert!(!late);
    assert_eq!(remaining, None);
}

#[tokio::test]
async fn a_sealed_secret_for_another_client_or_deployment_is_not_stored() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "mine").await?;
        let other = seeded(&tx, "other").await?;
        autonomous(&repository, dep.id, "agent-a-abc123").await?;
        let asked = request("agent-a-abc123", Duration::minutes(15));
        let now = Utc::now();
        repository.insert_request(dep.id, &asked, now).await?;

        let wrong_client = repository
            .store_sealed(
                dep.id,
                asked.request_id,
                &id("agent-b-abc123"),
                "c2VhbGVk",
                now,
            )
            .await?;
        let wrong_deployment = repository
            .store_sealed(
                other.id,
                asked.request_id,
                &id("agent-a-abc123"),
                "c2VhbGVk",
                now,
            )
            .await?;
        let state = repository
            .credential_state(other.id, asked.request_id, now)
            .await?;
        Ok((wrong_client, wrong_deployment, state))
    })
    .await;

    let (wrong_client, wrong_deployment, state) = found.expect("the transaction");
    assert!(!wrong_client);
    assert!(!wrong_deployment);
    assert_eq!(state, CredentialState::Missing);
}

#[tokio::test]
async fn spec_ar_6_a_credential_past_its_fifteen_minutes_is_unreadable_and_purged() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "expiry").await?;
        autonomous(&repository, dep.id, "agent-a-abc123").await?;
        let stale = request("agent-a-abc123", Duration::minutes(-1));
        let live = request("agent-a-abc123", Duration::minutes(15));
        let now = Utc::now();
        repository.insert_request(dep.id, &stale, now).await?;
        repository.insert_request(dep.id, &live, now).await?;
        repository
            .store_sealed(
                dep.id,
                stale.request_id,
                &id("agent-a-abc123"),
                "c2VhbGVk",
                now - Duration::minutes(20),
            )
            .await?;

        let state = repository
            .credential_state(dep.id, stale.request_id, now)
            .await?;
        let taken = repository
            .take_sealed(dep.id, stale.request_id, now)
            .await?;
        let open = repository.open_requests(dep.id, now).await?;
        let purged = repository.purge_expired(now).await?;
        let live_state = repository
            .credential_state(dep.id, live.request_id, now)
            .await?;
        Ok((state, taken, open, purged, live_state, live.request_id))
    })
    .await;

    let (state, taken, open, purged, live_state, live_id) = found.expect("the transaction");
    assert_eq!(state, CredentialState::Missing);
    assert!(taken.is_none());
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].request_id, live_id);
    assert_eq!(purged, 1);
    assert_eq!(live_state, CredentialState::Pending);
}

#[tokio::test]
async fn deleting_an_autonomous_agent_takes_its_credential_requests_with_it() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "delete").await?;
        autonomous(&repository, dep.id, "agent-a-abc123").await?;
        let asked = request("agent-a-abc123", Duration::minutes(15));
        let now = Utc::now();
        repository.insert_request(dep.id, &asked, now).await?;

        repository
            .delete_autonomous_agent(dep.id, &id("agent-a-abc123"))
            .await?;

        Ok((
            repository.autonomous_agents(dep.id).await?,
            repository
                .credential_state(dep.id, asked.request_id, now)
                .await?,
        ))
    })
    .await;

    let (agents, state) = found.expect("the transaction");
    assert!(agents.is_empty());
    assert_eq!(state, CredentialState::Missing);
}

#[tokio::test]
async fn autonomous_agents_are_listed_oldest_first_with_their_description() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let repository = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "list").await?;
        autonomous(&repository, dep.id, "agent-a-abc123").await?;
        autonomous(&repository, dep.id, "agent-b-abc123").await?;
        repository.autonomous_agents(dep.id).await
    })
    .await;

    let agents = found.expect("the transaction");
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].description.as_deref(), Some("nightly"));
}
