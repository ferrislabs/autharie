use autharie_auth::Identity;
use chrono::Utc;
use serde_json::{Value, json};

use crate::{
    CoreError,
    action::ports::ActionRepository,
    agents::{
        AGENT_SERVER_CREATED_AUDIT, AGENT_SERVER_DELETED_AUDIT, AGENT_SERVER_UPDATED_AUDIT,
        AgentError,
        entities::AgentSettings,
        ports::{AgentAccess, AgentRepository},
        server_ports::{AgentServerRepository, AgentServerService},
        servers::{CustomerServer, MAX_SERVERS, ServerId, ServerInput},
        service::AgentServiceImpl,
    },
    audit::{AuditChange, ports::AuditRepository},
    deployments::{Deployment, DeploymentId, ports::DeploymentRepository},
    generate_uuid_v7,
    organisation::{OrganisationId, ports::OrganisationRepository},
    user::ports::UserRepository,
};

fn summary(server: &CustomerServer) -> Value {
    json!({
        "deployment_id": server.deployment_id.0,
        "slug": server.slug,
        "name": server.name,
        "url": server.url,
        "enabled": server.enabled,
    })
}

impl<D, O, G, A, AU, U, P> AgentServiceImpl<D, O, G, A, AU, U, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    G: AgentRepository + AgentServerRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: AgentAccess,
{
    async fn stored_server(
        &self,
        deployment_id: DeploymentId,
        server_id: ServerId,
    ) -> Result<CustomerServer, CoreError> {
        self.publisher
            .agents
            .get_server(deployment_id, server_id)
            .await?
            .ok_or_else(|| AgentError::ServerNotFound { id: server_id.0 }.into())
    }

    async fn gateway_url(&self, deployment: &Deployment) -> Result<Option<String>, CoreError> {
        let settings = self
            .publisher
            .agents
            .settings(deployment.id)
            .await?
            .unwrap_or_else(|| AgentSettings::unset(deployment.id));

        Ok(self.mcp_url(deployment, &settings))
    }
}

impl<D, O, G, A, AU, U, P> AgentServerService for AgentServiceImpl<D, O, G, A, AU, U, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    G: AgentRepository + AgentServerRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: AgentAccess,
{
    async fn list_agent_servers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<CustomerServer>, CoreError> {
        self.policy
            .can_view_agents(identity, organisation_id)
            .await?;
        let deployment = self.deployment(organisation_id, deployment_id).await?;
        let gateway = self.gateway_url(&deployment).await?;

        Ok(self
            .publisher
            .agents
            .list_servers(deployment_id)
            .await?
            .into_iter()
            .map(|server| server.with_mcp_url(gateway.as_deref()))
            .collect())
    }

    async fn register_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: ServerInput,
    ) -> Result<CustomerServer, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;
        let slug = input
            .slug
            .ok_or_else(|| AgentError::invalid("slug: is required"))?;

        let known = self.publisher.agents.list_servers(deployment_id).await?;
        if known.len() >= MAX_SERVERS {
            return Err(AgentError::TooManyServers { max: MAX_SERVERS }.into());
        }
        if known.iter().any(|server| server.slug == slug) {
            return Err(AgentError::ServerSlugTaken {
                slug: slug.to_string(),
            }
            .into());
        }

        let now = Utc::now();
        self.publisher
            .agents
            .next_revision(deployment_id, now)
            .await?;
        let server = CustomerServer {
            id: ServerId(generate_uuid_v7()),
            deployment_id,
            slug,
            name: input.name,
            url: input.url,
            enabled: input.enabled,
            mcp_url: None,
            discovery: None,
            discovery_requested_at: Some(now),
            created_at: now,
            updated_at: now,
        };

        self.publisher.agents.insert_server(&server).await?;
        self.publisher.publish(&deployment, mcp_enabled).await?;
        self.write_audit(
            &identity,
            organisation_id,
            AGENT_SERVER_CREATED_AUDIT,
            "agent_server",
            server.id.0,
            AuditChange::new(Value::Null, summary(&server))?,
        )
        .await?;

        let gateway = self.gateway_url(&deployment).await?;
        Ok(server.with_mcp_url(gateway.as_deref()))
    }

    async fn update_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
        input: ServerInput,
    ) -> Result<CustomerServer, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;
        let before = self.stored_server(deployment_id, server_id).await?;

        let now = Utc::now();
        self.publisher
            .agents
            .next_revision(deployment_id, now)
            .await?;
        let moved = before.url != input.url;
        let server = CustomerServer {
            name: input.name,
            url: input.url,
            enabled: input.enabled,
            discovery: if moved {
                None
            } else {
                before.discovery.clone()
            },
            discovery_requested_at: if moved {
                Some(now)
            } else {
                before.discovery_requested_at
            },
            updated_at: now,
            ..before.clone()
        };

        if !self.publisher.agents.update_server(&server).await? {
            return Err(AgentError::ServerNotFound { id: server_id.0 }.into());
        }
        self.publisher.publish(&deployment, mcp_enabled).await?;
        self.write_audit(
            &identity,
            organisation_id,
            AGENT_SERVER_UPDATED_AUDIT,
            "agent_server",
            server_id.0,
            AuditChange::new(summary(&before), summary(&server))?,
        )
        .await?;

        let gateway = self.gateway_url(&deployment).await?;
        Ok(server.with_mcp_url(gateway.as_deref()))
    }

    async fn remove_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;
        let before = self.stored_server(deployment_id, server_id).await?;

        self.publisher
            .agents
            .next_revision(deployment_id, Utc::now())
            .await?;
        if !self
            .publisher
            .agents
            .delete_server(deployment_id, server_id)
            .await?
        {
            return Err(AgentError::ServerNotFound { id: server_id.0 }.into());
        }
        self.publisher.publish(&deployment, mcp_enabled).await?;

        self.write_audit(
            &identity,
            organisation_id,
            AGENT_SERVER_DELETED_AUDIT,
            "agent_server",
            server_id.0,
            AuditChange::new(summary(&before), Value::Null)?,
        )
        .await
    }

    async fn discover_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_agents(identity, organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;
        let before = self.stored_server(deployment_id, server_id).await?;

        let now = Utc::now();
        self.publisher
            .agents
            .next_revision(deployment_id, now)
            .await?;
        let server = CustomerServer {
            discovery_requested_at: Some(now),
            ..before
        };

        if !self.publisher.agents.update_server(&server).await? {
            return Err(AgentError::ServerNotFound { id: server_id.0 }.into());
        }
        self.publisher.publish(&deployment, mcp_enabled).await
    }
}
