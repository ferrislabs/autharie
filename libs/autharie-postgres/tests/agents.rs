use autharie_domain::{
    CoreError,
    agents::{
        AgentPolicy, AgentSettingsInput, AgentStatus, AgentsPhase, PolicyId, PolicyPhase,
        ports::AgentRepository,
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
use chrono::Utc;
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

fn status(phase: AgentsPhase) -> AgentStatus {
    AgentStatus {
        phase,
        gateway_client_ready: true,
        message: None,
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn settings_are_saved_and_every_save_bumps_the_revision() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "one").await?;
        let none = agents.settings(dep.id).await?;
        let input = AgentSettingsInput::parse(true, Some("acme")).unwrap();
        let first = agents.save_settings(dep.id, &input, Utc::now()).await?;
        let second = agents.save_settings(dep.id, &input, Utc::now()).await?;
        let third = agents.next_revision(dep.id, Utc::now()).await?;
        let read = agents.settings(dep.id).await?;
        Ok((none, first, second, third, read))
    })
    .await;

    let (none, first, second, third, read) = found.expect("the transaction");
    assert!(none.is_none());
    assert_eq!((first.revision, second.revision, third), (1, 2, 3));
    let read = read.expect("settings");
    assert!(read.enabled);
    assert_eq!(read.realm, "acme");
    assert_eq!(read.revision, 3);
    assert!(read.status.is_none());
}

#[tokio::test]
async fn the_first_revision_of_a_deployment_without_settings_creates_them_off() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "two").await?;
        let revision = agents.next_revision(dep.id, Utc::now()).await?;
        Ok((revision, agents.settings(dep.id).await?))
    })
    .await;

    let (revision, settings) = found.expect("the transaction");
    let settings = settings.expect("settings");
    assert_eq!(revision, 1);
    assert!(!settings.enabled);
    assert_eq!(settings.realm, "master");
}

#[tokio::test]
async fn policies_round_trip_with_their_rule_and_stay_with_their_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "three").await?;
        let other = seeded(&tx, "three-other").await?;
        let stored = policy(dep.id, "a", true, 1);
        agents.insert_policy(&stored).await?;
        agents.insert_policy(&policy(dep.id, "b", false, 2)).await?;
        agents
            .insert_policy(&policy(other.id, "c", true, 1))
            .await?;

        let listed = agents.list_policies(dep.id).await?;
        let read = agents.get_policy(dep.id, stored.id).await?;
        let wrong_deployment = agents.get_policy(other.id, stored.id).await?;
        Ok((stored, listed, read, wrong_deployment))
    })
    .await;

    let (stored, listed, read, wrong) = found.expect("the transaction");
    assert_eq!(
        listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    let read = read.expect("the policy");
    assert_eq!(read.rule, stored.rule);
    assert_eq!(read.cedar, stored.cedar);
    assert_eq!(read.phase, Some(PolicyPhase::Pending));
    assert_eq!(listed[1].phase, None);
    assert!(wrong.is_none());
}

#[tokio::test]
async fn a_policy_is_updated_and_deleted_only_within_its_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "four").await?;
        let other = seeded(&tx, "four-other").await?;
        let mut stored = policy(dep.id, "a", true, 1);
        agents.insert_policy(&stored).await?;

        stored.name = "renamed".to_string();
        stored.enabled = false;
        stored.phase = None;
        stored.rule = None;
        let updated = agents.update_policy(&stored).await?;
        let read = agents.get_policy(dep.id, stored.id).await?.unwrap();

        let deleted_elsewhere = agents.delete_policy(other.id, stored.id).await?;
        let deleted = agents.delete_policy(dep.id, stored.id).await?;
        let deleted_twice = agents.delete_policy(dep.id, stored.id).await?;
        Ok((updated, read, deleted_elsewhere, deleted, deleted_twice))
    })
    .await;

    let (updated, read, elsewhere, deleted, twice) = found.expect("the transaction");
    assert!(updated);
    assert_eq!(read.name, "renamed");
    assert!(!read.enabled);
    assert_eq!(read.rule, None);
    assert_eq!(read.phase, None);
    assert!(!elsewhere && deleted && !twice);
}

#[tokio::test]
async fn a_stale_status_is_not_recorded_and_a_newer_one_is() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "five").await?;
        for _ in 0..3 {
            agents.next_revision(dep.id, Utc::now()).await?;
        }

        let newer = agents
            .record_status(
                dep.id,
                2,
                &status(AgentsPhase::Ready),
                Some("https://a/mcp"),
            )
            .await?;
        let stale = agents
            .record_status(dep.id, 1, &status(AgentsPhase::Failed), None)
            .await?;
        let future = agents
            .record_status(dep.id, 9, &status(AgentsPhase::Failed), None)
            .await?;
        let same = agents
            .record_status(
                dep.id,
                2,
                &status(AgentsPhase::Applying),
                Some("https://a/mcp"),
            )
            .await?;
        Ok((
            newer,
            stale,
            future,
            same,
            agents.settings(dep.id).await?.unwrap(),
        ))
    })
    .await;

    let (newer, stale, future, same, settings) = found.expect("the transaction");
    assert!(newer && !stale && !future && same);
    let status = settings.status.expect("a status");
    assert_eq!(status.phase, AgentsPhase::Applying);
    assert_eq!(settings.applied_revision, 2);
    assert_eq!(settings.mcp_url.as_deref(), Some("https://a/mcp"));
}

#[tokio::test]
async fn a_policy_outcome_is_ignored_for_a_revision_older_than_the_policy() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "six").await?;
        let stored = policy(dep.id, "a", true, 5);
        agents.insert_policy(&stored).await?;

        let old = agents
            .record_policy_outcome(dep.id, stored.id, 4, PolicyPhase::Applied, None, Utc::now())
            .await?;
        let current = agents
            .record_policy_outcome(
                dep.id,
                stored.id,
                5,
                PolicyPhase::Invalid,
                Some("unexpected token"),
                Utc::now(),
            )
            .await?;
        let repeat = agents
            .record_policy_outcome(
                dep.id,
                stored.id,
                6,
                PolicyPhase::Invalid,
                Some("unexpected token"),
                Utc::now(),
            )
            .await?;
        let read = agents.get_policy(dep.id, stored.id).await?.unwrap();
        Ok((old, current, repeat, read))
    })
    .await;

    let (old, current, repeat, read) = found.expect("the transaction");
    assert!(!old && current && !repeat);
    assert_eq!(read.phase, Some(PolicyPhase::Invalid));
    assert_eq!(read.message.as_deref(), Some("unexpected token"));
}

#[tokio::test]
async fn deleting_the_deployment_row_takes_its_agents_with_it() {
    let Some(pool) = pool().await else {
        return;
    };

    let found: Result<_, CoreError> = in_scratch_tx(&pool, db_error, async |tx| {
        let agents = PostgresAgentRepository::new(&tx);
        let dep = seeded(&tx, "seven").await?;
        agents.next_revision(dep.id, Utc::now()).await?;
        agents.insert_policy(&policy(dep.id, "a", true, 1)).await?;

        {
            let mut guard = tx.lock().await;
            sqlx::query("DELETE FROM deployments WHERE id = $1")
                .bind(dep.id.0)
                .execute(&mut ***guard)
                .await
                .map_err(db_error)?;
        }

        Ok((
            agents.settings(dep.id).await?,
            agents.list_policies(dep.id).await?,
        ))
    })
    .await;

    let (settings, policies) = found.expect("the transaction");
    assert!(settings.is_none() && policies.is_empty());
}
