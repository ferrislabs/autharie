use autharie_domain::{
    CoreError,
    agents::{
        AgentPolicy, AgentSettings, AgentSettingsInput, AgentStatus, AgentsPhase, PolicyId,
        PolicyPhase, ports::AgentRepository,
    },
    deployments::DeploymentId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

pub(super) fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {what}: {error}"),
    }
}

pub(super) fn corrupt(what: impl std::fmt::Display) -> CoreError {
    CoreError::InternalError(format!("stored agent data cannot be read: {what}"))
}

fn revision_column(revision: u64) -> Result<i64, CoreError> {
    i64::try_from(revision)
        .map_err(|_| CoreError::InternalError("a revision does not fit the column".to_string()))
}

struct SettingsRow {
    deployment_id: Uuid,
    enabled: bool,
    realm: String,
    revision: i64,
    applied_revision: i64,
    phase: Option<String>,
    gateway_client_ready: bool,
    mcp_url: Option<String>,
    message: Option<String>,
    updated_at: DateTime<Utc>,
}

impl SettingsRow {
    fn into_settings(self) -> Result<AgentSettings, CoreError> {
        let status = match self.phase {
            None => None,
            Some(phase) => Some(AgentStatus {
                phase: AgentsPhase::parse(&phase)
                    .ok_or_else(|| corrupt(format!("phase '{phase}'")))?,
                gateway_client_ready: self.gateway_client_ready,
                message: self.message,
                updated_at: self.updated_at,
            }),
        };

        Ok(AgentSettings {
            deployment_id: DeploymentId(self.deployment_id),
            enabled: self.enabled,
            realm: self.realm,
            revision: u64::try_from(self.revision).map_err(corrupt)?,
            applied_revision: u64::try_from(self.applied_revision).map_err(corrupt)?,
            mcp_url: self.mcp_url,
            status,
        })
    }
}

struct PolicyRow {
    id: Uuid,
    deployment_id: Uuid,
    name: String,
    description: Option<String>,
    enabled: bool,
    rule: Option<Value>,
    cedar: String,
    phase: Option<String>,
    message: Option<String>,
    desired_revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl PolicyRow {
    fn into_policy(self) -> Result<AgentPolicy, CoreError> {
        Ok(AgentPolicy {
            id: PolicyId(self.id),
            deployment_id: DeploymentId(self.deployment_id),
            name: self.name,
            description: self.description,
            enabled: self.enabled,
            rule: self.rule,
            cedar: self.cedar,
            phase: self
                .phase
                .map(|phase| {
                    PolicyPhase::parse(&phase).ok_or_else(|| corrupt(format!("phase '{phase}'")))
                })
                .transpose()?,
            message: self.message,
            desired_revision: u64::try_from(self.desired_revision).map_err(corrupt)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Agent, backend = Postgres)]
pub struct PostgresAgentRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresAgentRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }

    pub(super) fn tx(&self) -> &SharedTx<'tx> {
        &self.tx
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl AgentRepository for PostgresAgentRepository<'_> {
    async fn settings(
        &self,
        deployment_id: DeploymentId,
    ) -> Result<Option<AgentSettings>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SettingsRow,
                r#"
            SELECT deployment_id, enabled, realm, revision, applied_revision, phase,
                   gateway_client_ready, mcp_url, message, updated_at
            FROM deployment_agents
            WHERE deployment_id = $1
            "#,
                deployment_id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read the agent settings"))?;

        row.map(SettingsRow::into_settings).transpose()
    }

    async fn save_settings(
        &self,
        deployment_id: DeploymentId,
        input: &AgentSettingsInput,
        at: DateTime<Utc>,
    ) -> Result<AgentSettings, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SettingsRow,
                r#"
            INSERT INTO deployment_agents (deployment_id, enabled, realm, revision, updated_at)
            VALUES ($1, $2, $3, 1, $4)
            ON CONFLICT (deployment_id) DO UPDATE
            SET enabled = $2,
                realm = $3,
                revision = deployment_agents.revision + 1
            RETURNING deployment_id, enabled, realm, revision, applied_revision, phase,
                      gateway_client_ready, mcp_url, message, updated_at
            "#,
                deployment_id.0,
                input.enabled,
                input.realm,
                at
            )
            .fetch_one(&mut ***tx)
            .await
        }
        .map_err(db("save the agent settings"))?;

        row.into_settings()
    }

    async fn next_revision(
        &self,
        deployment_id: DeploymentId,
        at: DateTime<Utc>,
    ) -> Result<u64, CoreError> {
        let revision = {
            let mut tx = self.tx.lock().await;
            sqlx::query_scalar!(
                r#"
            INSERT INTO deployment_agents (deployment_id, revision, updated_at)
            VALUES ($1, 1, $2)
            ON CONFLICT (deployment_id) DO UPDATE
            SET revision = deployment_agents.revision + 1
            RETURNING revision
            "#,
                deployment_id.0,
                at
            )
            .fetch_one(&mut ***tx)
            .await
        }
        .map_err(db("take the next agents revision"))?;

        u64::try_from(revision).map_err(corrupt)
    }

    async fn record_status(
        &self,
        deployment_id: DeploymentId,
        revision: u64,
        status: &AgentStatus,
        mcp_url: Option<&str>,
    ) -> Result<bool, CoreError> {
        let revision = revision_column(revision)?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_agents
            SET applied_revision = $2,
                phase = $3,
                gateway_client_ready = $4,
                mcp_url = $5,
                message = $6,
                updated_at = $7
            WHERE deployment_id = $1
              AND applied_revision <= $2
              AND revision >= $2
            "#,
                deployment_id.0,
                revision,
                status.phase.as_str(),
                status.gateway_client_ready,
                mcp_url,
                status.message,
                status.updated_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record the agents status"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn list_policies(
        &self,
        deployment_id: DeploymentId,
    ) -> Result<Vec<AgentPolicy>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                PolicyRow,
                r#"
            SELECT id, deployment_id, name, description, enabled, rule, cedar, phase, message,
                   desired_revision, created_at, updated_at
            FROM agent_policies
            WHERE deployment_id = $1
            ORDER BY created_at, id
            "#,
                deployment_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list agent policies"))?;

        rows.into_iter().map(PolicyRow::into_policy).collect()
    }

    async fn get_policy(
        &self,
        deployment_id: DeploymentId,
        id: PolicyId,
    ) -> Result<Option<AgentPolicy>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                PolicyRow,
                r#"
            SELECT id, deployment_id, name, description, enabled, rule, cedar, phase, message,
                   desired_revision, created_at, updated_at
            FROM agent_policies
            WHERE deployment_id = $1 AND id = $2
            "#,
                deployment_id.0,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read an agent policy"))?;

        row.map(PolicyRow::into_policy).transpose()
    }

    async fn insert_policy(&self, policy: &AgentPolicy) -> Result<(), CoreError> {
        let revision = revision_column(policy.desired_revision)?;

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO agent_policies (
                id, deployment_id, name, description, enabled, rule, cedar, phase, message,
                desired_revision, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
                policy.id.0,
                policy.deployment_id.0,
                policy.name,
                policy.description,
                policy.enabled,
                policy.rule.as_ref() as Option<&Value>,
                policy.cedar,
                policy.phase.map(|phase| phase.as_str()),
                policy.message,
                revision,
                policy.created_at,
                policy.updated_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store an agent policy"))?;

        Ok(())
    }

    async fn update_policy(&self, policy: &AgentPolicy) -> Result<bool, CoreError> {
        let revision = revision_column(policy.desired_revision)?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE agent_policies
            SET name = $3,
                description = $4,
                enabled = $5,
                rule = $6,
                cedar = $7,
                phase = $8,
                message = $9,
                desired_revision = $10,
                updated_at = $11
            WHERE deployment_id = $1 AND id = $2
            "#,
                policy.deployment_id.0,
                policy.id.0,
                policy.name,
                policy.description,
                policy.enabled,
                policy.rule.as_ref() as Option<&Value>,
                policy.cedar,
                policy.phase.map(|phase| phase.as_str()),
                policy.message,
                revision,
                policy.updated_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("change an agent policy"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_policy(
        &self,
        deployment_id: DeploymentId,
        id: PolicyId,
    ) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "DELETE FROM agent_policies WHERE deployment_id = $1 AND id = $2",
                deployment_id.0,
                id.0
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("delete an agent policy"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn record_policy_outcome(
        &self,
        deployment_id: DeploymentId,
        id: PolicyId,
        revision: u64,
        phase: PolicyPhase,
        message: Option<&str>,
        _at: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let revision = revision_column(revision)?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE agent_policies
            SET phase = $4,
                message = $5
            WHERE deployment_id = $1
              AND id = $2
              AND enabled
              AND desired_revision <= $3
              AND (phase IS DISTINCT FROM $4 OR message IS DISTINCT FROM $5)
            "#,
                deployment_id.0,
                id.0,
                revision,
                phase.as_str(),
                message
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record an agent policy outcome"))?;

        Ok(result.rows_affected() > 0)
    }
}
