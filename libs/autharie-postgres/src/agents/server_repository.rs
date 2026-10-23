use autharie_domain::{
    CoreError,
    agents::{
        AgentError,
        server_ports::AgentServerRepository,
        servers::{CustomerServer, CustomerServerUrl, Discovery, ServerId, ServerSlug},
    },
    deployments::DeploymentId,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::PostgresAgentRepository;

fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {what}: {error}"),
    }
}

fn corrupt(what: impl std::fmt::Display) -> CoreError {
    CoreError::InternalError(format!("stored MCP server data cannot be read: {what}"))
}

struct ServerRow {
    id: Uuid,
    deployment_id: Uuid,
    slug: String,
    name: String,
    url: String,
    enabled: bool,
    discovery: Option<Value>,
    discovery_requested_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ServerRow {
    fn into_server(self) -> Result<CustomerServer, CoreError> {
        Ok(CustomerServer {
            id: ServerId(self.id),
            deployment_id: DeploymentId(self.deployment_id),
            slug: ServerSlug::try_from(self.slug).map_err(corrupt)?,
            name: self.name,
            url: CustomerServerUrl::try_from(self.url).map_err(corrupt)?,
            enabled: self.enabled,
            mcp_url: None,
            discovery: self
                .discovery
                .map(serde_json::from_value::<Discovery>)
                .transpose()
                .map_err(corrupt)?,
            discovery_requested_at: self.discovery_requested_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn discovery_column(discovery: Option<&Discovery>) -> Result<Option<Value>, CoreError> {
    discovery
        .map(serde_json::to_value)
        .transpose()
        .map_err(corrupt)
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl AgentServerRepository for PostgresAgentRepository<'_> {
    async fn list_servers(
        &self,
        deployment_id: DeploymentId,
    ) -> Result<Vec<CustomerServer>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                ServerRow,
                r#"
            SELECT id, deployment_id, slug, name, url, enabled, discovery,
                   discovery_requested_at, created_at, updated_at
            FROM agent_servers
            WHERE deployment_id = $1
            ORDER BY created_at, id
            "#,
                deployment_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list MCP servers"))?;

        rows.into_iter().map(ServerRow::into_server).collect()
    }

    async fn get_server(
        &self,
        deployment_id: DeploymentId,
        id: ServerId,
    ) -> Result<Option<CustomerServer>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                ServerRow,
                r#"
            SELECT id, deployment_id, slug, name, url, enabled, discovery,
                   discovery_requested_at, created_at, updated_at
            FROM agent_servers
            WHERE deployment_id = $1 AND id = $2
            "#,
                deployment_id.0,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read an MCP server"))?;

        row.map(ServerRow::into_server).transpose()
    }

    async fn insert_server(&self, server: &CustomerServer) -> Result<(), CoreError> {
        let discovery = discovery_column(server.discovery.as_ref())?;

        let inserted = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO agent_servers (
                id, deployment_id, slug, name, url, enabled, discovery,
                discovery_requested_at, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
                server.id.0,
                server.deployment_id.0,
                server.slug.as_str(),
                server.name,
                server.url.as_str(),
                server.enabled,
                discovery.as_ref() as Option<&Value>,
                server.discovery_requested_at,
                server.created_at,
                server.updated_at
            )
            .execute(&mut ***tx)
            .await
        };

        match inserted {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(AgentError::ServerSlugTaken {
                    slug: server.slug.to_string(),
                }
                .into())
            }
            Err(error) => Err(db("store an MCP server")(error)),
        }
    }

    async fn update_server(&self, server: &CustomerServer) -> Result<bool, CoreError> {
        let discovery = discovery_column(server.discovery.as_ref())?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE agent_servers
            SET name = $3,
                url = $4,
                enabled = $5,
                discovery = $6,
                discovery_requested_at = $7,
                updated_at = $8
            WHERE deployment_id = $1 AND id = $2
            "#,
                server.deployment_id.0,
                server.id.0,
                server.name,
                server.url.as_str(),
                server.enabled,
                discovery.as_ref() as Option<&Value>,
                server.discovery_requested_at,
                server.updated_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("change an MCP server"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_server(
        &self,
        deployment_id: DeploymentId,
        id: ServerId,
    ) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "DELETE FROM agent_servers WHERE deployment_id = $1 AND id = $2",
                deployment_id.0,
                id.0
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("delete an MCP server"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn record_discovery(
        &self,
        deployment_id: DeploymentId,
        id: ServerId,
        discovery: &Discovery,
    ) -> Result<bool, CoreError> {
        let stored = discovery_column(Some(discovery))?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE agent_servers
            SET discovery = $3
            WHERE deployment_id = $1
              AND id = $2
              AND (discovery_requested_at IS NULL OR $4 >= discovery_requested_at)
              AND (discovery IS NULL OR $4 > (discovery ->> 'discovered_at')::timestamptz)
            "#,
                deployment_id.0,
                id.0,
                stored.as_ref() as Option<&Value>,
                discovery.discovered_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record the tools an MCP server offers"))?;

        Ok(result.rows_affected() > 0)
    }
}
