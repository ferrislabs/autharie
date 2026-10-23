use autharie_domain::{
    CoreError,
    agents::{
        registry::{
            AccessLists, AgentClientId, AutonomousAgent, ClientKind, ClientSource,
            CredentialRequest, CredentialState, ReportedClient, ReportedRegistry, RequestKind,
            SealedCredential, SealingJwk, SealingKey,
        },
        registry_ports::AgentRegistryRepository,
    },
    deployments::DeploymentId,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::agent_repository::{PostgresAgentRepository, corrupt, db};

struct ClientRow {
    client_id: String,
    name: Option<String>,
    source: String,
    kind: String,
    enabled: bool,
    created_at: Option<DateTime<Utc>>,
}

impl ClientRow {
    fn into_client(self) -> Result<ReportedClient, CoreError> {
        Ok(ReportedClient {
            client_id: AgentClientId::try_from(self.client_id).map_err(corrupt)?,
            name: self.name,
            source: ClientSource::parse(&self.source)
                .ok_or_else(|| corrupt(format!("source '{}'", self.source)))?,
            kind: ClientKind::parse(&self.kind)
                .ok_or_else(|| corrupt(format!("kind '{}'", self.kind)))?,
            enabled: self.enabled,
            created_at: self.created_at,
        })
    }
}

struct RequestRow {
    request_id: Uuid,
    client_id: String,
    kind: String,
    sealing_key: Value,
    expires_at: DateTime<Utc>,
}

impl RequestRow {
    fn into_request(self) -> Result<CredentialRequest, CoreError> {
        let jwk: SealingJwk = serde_json::from_value(self.sealing_key).map_err(corrupt)?;

        Ok(CredentialRequest {
            request_id: self.request_id,
            client_id: AgentClientId::try_from(self.client_id).map_err(corrupt)?,
            kind: RequestKind::parse(&self.kind)
                .ok_or_else(|| corrupt(format!("request kind '{}'", self.kind)))?,
            sealing_key: SealingKey::try_from(jwk).map_err(corrupt)?,
            expires_at: self.expires_at,
        })
    }
}

fn position(index: usize) -> Result<i32, CoreError> {
    i32::try_from(index).map_err(|_| corrupt("too many clients"))
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl AgentRegistryRepository for PostgresAgentRepository<'_> {
    async fn access(&self, deployment_id: DeploymentId) -> Result<AccessLists, CoreError> {
        let row = {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            SELECT approval_required, approved, blocked, deleted
            FROM agent_access
            WHERE deployment_id = $1
            "#,
                deployment_id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read the agent access lists"))?;

        Ok(row.map_or_else(AccessLists::default, |row| AccessLists {
            approval_required: row.approval_required,
            approved: row.approved.into_iter().collect(),
            blocked: row.blocked.into_iter().collect(),
            deleted: row.deleted.into_iter().collect(),
        }))
    }

    async fn save_access(
        &self,
        deployment_id: DeploymentId,
        access: &AccessLists,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let approved: Vec<String> = access.approved.iter().cloned().collect();
        let blocked: Vec<String> = access.blocked.iter().cloned().collect();
        let deleted: Vec<String> = access.deleted.iter().cloned().collect();

        {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            INSERT INTO agent_access (
                deployment_id, approval_required, approved, blocked, deleted, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (deployment_id) DO UPDATE
            SET approval_required = $2,
                approved = $3,
                blocked = $4,
                deleted = $5,
                updated_at = $6
            "#,
                deployment_id.0,
                access.approval_required,
                &approved,
                &blocked,
                &deleted,
                at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("save the agent access lists"))?;

        Ok(())
    }

    async fn registry(&self, deployment_id: DeploymentId) -> Result<ReportedRegistry, CoreError> {
        let mut tx = self.tx().lock().await;
        let rows = sqlx::query_as!(
            ClientRow,
            r#"
            SELECT client_id, name, source, kind, enabled, created_at
            FROM agent_clients
            WHERE deployment_id = $1
            ORDER BY position, client_id
            "#,
            deployment_id.0
        )
        .fetch_all(&mut ***tx)
        .await
        .map_err(db("read the agent registry"))?;
        let truncated = sqlx::query_scalar!(
            "SELECT clients_truncated FROM agent_access WHERE deployment_id = $1",
            deployment_id.0
        )
        .fetch_optional(&mut ***tx)
        .await
        .map_err(db("read the agent registry"))?
        .unwrap_or(false);
        drop(tx);

        Ok(ReportedRegistry {
            clients: rows
                .into_iter()
                .map(ClientRow::into_client)
                .collect::<Result<_, _>>()?,
            truncated,
        })
    }

    async fn replace_registry(
        &self,
        deployment_id: DeploymentId,
        registry: &ReportedRegistry,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let ids: Vec<String> = registry
            .clients
            .iter()
            .map(|client| client.client_id.to_string())
            .collect();
        let names: Vec<Option<String>> = registry
            .clients
            .iter()
            .map(|client| client.name.clone())
            .collect();
        let sources: Vec<String> = registry
            .clients
            .iter()
            .map(|client| client.source.as_str().to_string())
            .collect();
        let kinds: Vec<String> = registry
            .clients
            .iter()
            .map(|client| client.kind.as_str().to_string())
            .collect();
        let enabled: Vec<bool> = registry
            .clients
            .iter()
            .map(|client| client.enabled)
            .collect();
        let created: Vec<Option<DateTime<Utc>>> = registry
            .clients
            .iter()
            .map(|client| client.created_at)
            .collect();
        let positions: Vec<i32> = (0..registry.clients.len())
            .map(position)
            .collect::<Result<_, _>>()?;

        let mut tx = self.tx().lock().await;
        sqlx::query("DELETE FROM agent_clients WHERE deployment_id = $1 AND client_id <> ALL($2)")
            .bind(deployment_id.0)
            .bind(&ids)
            .execute(&mut ***tx)
            .await
            .map_err(db("replace the agent registry"))?;
        sqlx::query(
            r#"
            INSERT INTO agent_clients (
                deployment_id, client_id, name, source, kind, enabled, created_at,
                reported_at, position
            )
            SELECT $1, client_id, name, source, kind, enabled, created_at, $8, position
            FROM UNNEST(
                $2::text[], $3::text[], $4::text[], $5::text[], $6::bool[],
                $7::timestamptz[], $9::int[]
            ) AS reported(client_id, name, source, kind, enabled, created_at, position)
            ON CONFLICT (deployment_id, client_id) DO UPDATE
            SET name = EXCLUDED.name,
                source = EXCLUDED.source,
                kind = EXCLUDED.kind,
                enabled = EXCLUDED.enabled,
                created_at = EXCLUDED.created_at,
                reported_at = EXCLUDED.reported_at,
                position = EXCLUDED.position
            "#,
        )
        .bind(deployment_id.0)
        .bind(&ids)
        .bind(&names)
        .bind(&sources)
        .bind(&kinds)
        .bind(&enabled)
        .bind(&created)
        .bind(at)
        .bind(&positions)
        .execute(&mut ***tx)
        .await
        .map_err(db("replace the agent registry"))?;
        sqlx::query!(
            r#"
            INSERT INTO agent_access (deployment_id, clients_truncated, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (deployment_id) DO UPDATE SET clients_truncated = $2
            "#,
            deployment_id.0,
            registry.truncated,
            at
        )
        .execute(&mut ***tx)
        .await
        .map_err(db("replace the agent registry"))?;

        Ok(())
    }

    async fn autonomous_agents(
        &self,
        deployment_id: DeploymentId,
    ) -> Result<Vec<AutonomousAgent>, CoreError> {
        let rows = {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            SELECT client_id, name, description, created_at
            FROM agent_autonomous
            WHERE deployment_id = $1
            ORDER BY created_at, client_id
            "#,
                deployment_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list autonomous agents"))?;

        rows.into_iter()
            .map(|row| {
                Ok(AutonomousAgent {
                    client_id: AgentClientId::try_from(row.client_id).map_err(corrupt)?,
                    name: row.name,
                    description: row.description,
                    created_at: row.created_at,
                })
            })
            .collect()
    }

    async fn insert_autonomous_agent(
        &self,
        deployment_id: DeploymentId,
        agent: &AutonomousAgent,
    ) -> Result<(), CoreError> {
        {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            INSERT INTO agent_autonomous (deployment_id, client_id, name, description, created_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
                deployment_id.0,
                agent.client_id.as_str(),
                agent.name,
                agent.description,
                agent.created_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store an autonomous agent"))?;

        Ok(())
    }

    async fn delete_autonomous_agent(
        &self,
        deployment_id: DeploymentId,
        client_id: &AgentClientId,
    ) -> Result<(), CoreError> {
        let mut tx = self.tx().lock().await;
        sqlx::query!(
            "DELETE FROM agent_credentials WHERE deployment_id = $1 AND client_id = $2",
            deployment_id.0,
            client_id.as_str()
        )
        .execute(&mut ***tx)
        .await
        .map_err(db("delete an autonomous agent"))?;
        sqlx::query!(
            "DELETE FROM agent_autonomous WHERE deployment_id = $1 AND client_id = $2",
            deployment_id.0,
            client_id.as_str()
        )
        .execute(&mut ***tx)
        .await
        .map_err(db("delete an autonomous agent"))?;

        Ok(())
    }

    async fn insert_request(
        &self,
        deployment_id: DeploymentId,
        request: &CredentialRequest,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            INSERT INTO agent_credentials (
                request_id, deployment_id, client_id, kind, sealing_key, created_at, expires_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
                request.request_id,
                deployment_id.0,
                request.client_id.as_str(),
                request.kind.as_str(),
                request.sealing_key.payload() as Value,
                at,
                request.expires_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store a credential request"))?;

        Ok(())
    }

    async fn open_requests(
        &self,
        deployment_id: DeploymentId,
        now: DateTime<Utc>,
    ) -> Result<Vec<CredentialRequest>, CoreError> {
        let rows = {
            let mut tx = self.tx().lock().await;
            sqlx::query_as!(
                RequestRow,
                r#"
            SELECT request_id, client_id, kind, sealing_key, expires_at
            FROM agent_credentials
            WHERE deployment_id = $1
              AND sealed IS NULL
              AND delivered_at IS NULL
              AND expires_at > $2
            ORDER BY created_at, request_id
            "#,
                deployment_id.0,
                now
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list credential requests"))?;

        rows.into_iter().map(RequestRow::into_request).collect()
    }

    async fn store_sealed(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        client_id: &AgentClientId,
        sealed: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            UPDATE agent_credentials
            SET sealed = $4
            WHERE request_id = $1
              AND deployment_id = $2
              AND client_id = $3
              AND sealed IS NULL
              AND delivered_at IS NULL
              AND expires_at > $5
            "#,
                request_id,
                deployment_id.0,
                client_id.as_str(),
                sealed,
                now
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store a sealed credential"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn credential_state(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<CredentialState, CoreError> {
        let row = {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            SELECT sealed IS NOT NULL AS "ready!", delivered_at IS NOT NULL AS "delivered!"
            FROM agent_credentials
            WHERE request_id = $1 AND deployment_id = $2 AND expires_at > $3
            "#,
                request_id,
                deployment_id.0,
                now
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read a credential request"))?;

        Ok(match row {
            None => CredentialState::Missing,
            Some(row) if row.delivered => CredentialState::Delivered,
            Some(row) if row.ready => CredentialState::Ready,
            Some(_) => CredentialState::Pending,
        })
    }

    async fn take_sealed(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<SealedCredential>, CoreError> {
        let row = {
            let mut tx = self.tx().lock().await;
            sqlx::query!(
                r#"
            WITH old AS (
                SELECT request_id, client_id, sealed
                FROM agent_credentials
                WHERE request_id = $1
                  AND deployment_id = $2
                  AND sealed IS NOT NULL
                  AND delivered_at IS NULL
                  AND expires_at > $3
                FOR UPDATE
            )
            UPDATE agent_credentials AS credential
            SET sealed = NULL, delivered_at = $3
            FROM old
            WHERE credential.request_id = old.request_id
            RETURNING old.client_id AS "client_id!", old.sealed AS "sealed!"
            "#,
                request_id,
                deployment_id.0,
                now
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("take a sealed credential"))?;

        row.map(|row| {
            Ok(SealedCredential {
                client_id: AgentClientId::try_from(row.client_id).map_err(corrupt)?,
                sealed: row.sealed,
            })
        })
        .transpose()
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64, CoreError> {
        let result = {
            let mut tx = self.tx().lock().await;
            sqlx::query!("DELETE FROM agent_credentials WHERE expires_at <= $1", now)
                .execute(&mut ***tx)
                .await
        }
        .map_err(db("purge expired credentials"))?;

        Ok(result.rows_affected())
    }
}
