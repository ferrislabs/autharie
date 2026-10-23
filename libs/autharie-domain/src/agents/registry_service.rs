use autharie_auth::Identity;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    CoreError,
    action::ports::ActionRepository,
    agents::{
        AgentError, MAX_AUTONOMOUS_AGENTS_PER_DEPLOYMENT,
        entities::AgentSettings,
        ports::{AgentAccess, AgentRepository},
        registry::{
            AccessLists, AgentClientId, AutonomousAgent, AutonomousInput, CREDENTIAL_TTL_MINUTES,
            CredentialFetch, CredentialRequest, CredentialState, DeliveredCredential, RegistryView,
            RequestKind, SealingKey, assemble, token_url,
        },
        registry_ports::{AgentActivityIndex, AgentRegistryRepository, AgentRegistryService},
        service::AgentPublisher,
    },
    audit::{
        AuditAction, AuditChange, AuditEntry, AuditEntryId, AuditTarget, AuditTargetKind,
        ports::AuditRepository, service::audit_actor,
    },
    deployments::{Deployment, DeploymentId, DeploymentKind, ports::DeploymentRepository},
    generate_uuid_v7,
    organisation::{OrganisationId, ports::OrganisationRepository},
    user::ports::UserRepository,
};

pub const APPROVAL_MODE_AUDIT: &str = "deployment.agent_registry.approval_mode";
pub const AGENT_APPROVED_AUDIT: &str = "deployment.agent_registry.approved";
pub const AGENT_BLOCKED_AUDIT: &str = "deployment.agent_registry.blocked";
pub const AGENT_UNBLOCKED_AUDIT: &str = "deployment.agent_registry.unblocked";
pub const AGENT_DELETED_AUDIT: &str = "deployment.agent_registry.deleted";
pub const AUTONOMOUS_CREATED_AUDIT: &str = "deployment.autonomous_agent.created";
pub const AUTONOMOUS_ROTATED_AUDIT: &str = "deployment.autonomous_agent.rotated";

const ACTIVITY_DAYS: i64 = 30;

pub struct AgentRegistryServiceImpl<D, O, G, A, AU, U, P>
where
    A: ActionRepository,
{
    publisher: AgentPublisher<D, G, A>,
    organisations: O,
    audit: AU,
    users: U,
    policy: P,
    domain: Option<String>,
}

impl<D, O, G, A, AU, U, P> AgentRegistryServiceImpl<D, O, G, A, AU, U, P>
where
    A: ActionRepository,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        deployments: D,
        organisations: O,
        agents: G,
        actions: A,
        audit: AU,
        users: U,
        policy: P,
        domain: Option<String>,
    ) -> Self
    where
        D: DeploymentRepository,
        G: AgentRepository + AgentRegistryRepository,
    {
        Self {
            publisher: AgentPublisher::new(deployments, agents, actions),
            organisations,
            audit,
            users,
            policy,
            domain,
        }
    }
}

impl<D, O, G, A, AU, U, P> AgentRegistryServiceImpl<D, O, G, A, AU, U, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    G: AgentRepository + AgentRegistryRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: AgentAccess,
{
    async fn ferriskey_deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        let deployment = self
            .publisher
            .deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .filter(|deployment| deployment.deleted_at.is_none())
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })?;

        if deployment.kind != DeploymentKind::Ferriskey {
            return Err(AgentError::UnsupportedKind.into());
        }

        Ok(deployment)
    }

    async fn mcp_enabled(&self, organisation_id: OrganisationId) -> Result<bool, CoreError> {
        Ok(self
            .organisations
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })?
            .mcp_enabled)
    }

    async fn settings(&self, deployment_id: DeploymentId) -> Result<AgentSettings, CoreError> {
        Ok(self
            .publisher
            .agents
            .settings(deployment_id)
            .await?
            .unwrap_or_else(|| AgentSettings::unset(deployment_id)))
    }

    fn mcp_url(&self, deployment: &Deployment, settings: &AgentSettings) -> Option<String> {
        settings.mcp_url.clone().or_else(|| {
            self.domain.as_ref().map(|zone| {
                format!(
                    "https://{}/mcp",
                    crate::dns::hostname_for(&deployment.name.0, zone)
                )
            })
        })
    }

    async fn announce(&self, deployment: &Deployment, mcp_enabled: bool) -> Result<(), CoreError> {
        self.publisher
            .agents
            .next_revision(deployment.id, Utc::now())
            .await?;
        self.publisher.publish(deployment, mcp_enabled).await
    }

    async fn write_audit(
        &self,
        identity: &Identity,
        organisation_id: OrganisationId,
        action: &str,
        deployment_id: DeploymentId,
        change: AuditChange,
    ) -> Result<(), CoreError> {
        self.audit
            .append(AuditEntry::record(
                AuditEntryId(generate_uuid_v7()),
                organisation_id,
                audit_actor(identity, &self.users).await?,
                AuditAction(action.to_string()),
                AuditTarget {
                    kind: AuditTargetKind::Custom("deployment_agents".to_string()),
                    id: deployment_id.0,
                },
                Some(change),
                Utc::now(),
            ))
            .await
    }

    async fn change_access(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        audit: &str,
        change: impl FnOnce(&mut AccessLists),
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .ferriskey_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.mcp_enabled(organisation_id).await?;

        let before = self.publisher.agents.access(deployment_id).await?;
        let mut after = before.clone();
        change(&mut after);

        self.publisher
            .agents
            .save_access(deployment_id, &after, Utc::now())
            .await?;
        self.announce(&deployment, mcp_enabled).await?;

        self.write_audit(
            &identity,
            organisation_id,
            audit,
            deployment_id,
            AuditChange::new(before.payload(), after.payload())?,
        )
        .await
    }

    async fn open_request(
        &self,
        deployment: &Deployment,
        client_id: AgentClientId,
        kind: RequestKind,
        sealing_key: SealingKey,
    ) -> Result<Uuid, CoreError> {
        let now = Utc::now();
        let request = CredentialRequest {
            request_id: generate_uuid_v7(),
            client_id,
            kind,
            sealing_key,
            expires_at: now + Duration::minutes(CREDENTIAL_TTL_MINUTES),
        };
        self.publisher
            .agents
            .insert_request(deployment.id, &request, now)
            .await?;

        Ok(request.request_id)
    }
}

impl<D, O, G, A, AU, U, P> AgentRegistryService for AgentRegistryServiceImpl<D, O, G, A, AU, U, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    G: AgentRepository + AgentRegistryRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: AgentAccess,
{
    async fn list_agent_clients<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        activity: I,
    ) -> Result<RegistryView, CoreError>
    where
        I: AgentActivityIndex,
    {
        self.policy
            .can_view_agents(identity, organisation_id)
            .await?;
        self.ferriskey_deployment(organisation_id, deployment_id)
            .await?;

        let agents = &self.publisher.agents;
        let reported = agents.registry(deployment_id).await?;
        let access = agents.access(deployment_id).await?;
        let autonomous = agents.autonomous_agents(deployment_id).await?;
        let seen = activity
            .last_seen(
                organisation_id,
                deployment_id,
                Utc::now() - Duration::days(ACTIVITY_DAYS),
            )
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "the agents' last activity could not be read");
                Default::default()
            });

        Ok(assemble(&reported, &autonomous, &access, &seen))
    }

    async fn set_approval_mode(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        approval_required: bool,
    ) -> Result<(), CoreError> {
        self.change_access(
            identity,
            organisation_id,
            deployment_id,
            APPROVAL_MODE_AUDIT,
            |lists| lists.approval_required = approval_required,
        )
        .await
    }

    async fn approve_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        self.change_access(
            identity,
            organisation_id,
            deployment_id,
            AGENT_APPROVED_AUDIT,
            |lists| lists.approve(&client_id),
        )
        .await
    }

    async fn block_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        self.change_access(
            identity,
            organisation_id,
            deployment_id,
            AGENT_BLOCKED_AUDIT,
            |lists| lists.block(&client_id),
        )
        .await
    }

    async fn unblock_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        self.change_access(
            identity,
            organisation_id,
            deployment_id,
            AGENT_UNBLOCKED_AUDIT,
            |lists| lists.unblock(&client_id),
        )
        .await
    }

    async fn delete_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        self.ferriskey_deployment(organisation_id, deployment_id)
            .await?;
        self.publisher
            .agents
            .delete_autonomous_agent(deployment_id, &client_id)
            .await?;

        self.change_access(
            identity,
            organisation_id,
            deployment_id,
            AGENT_DELETED_AUDIT,
            |lists| lists.forget(&client_id),
        )
        .await
    }

    async fn create_autonomous_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: AutonomousInput,
    ) -> Result<Uuid, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .ferriskey_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.mcp_enabled(organisation_id).await?;
        if !self.settings(deployment_id).await?.enabled || !mcp_enabled {
            return Err(AgentError::NotEnabled.into());
        }

        let agents = &self.publisher.agents;
        if agents.autonomous_agents(deployment_id).await?.len()
            >= MAX_AUTONOMOUS_AGENTS_PER_DEPLOYMENT
        {
            return Err(AgentError::TooManyAutonomousAgents {
                max: MAX_AUTONOMOUS_AGENTS_PER_DEPLOYMENT,
            }
            .into());
        }

        let client_id = AgentClientId::generate(&input.name);
        let agent = AutonomousAgent {
            client_id: client_id.clone(),
            name: input.name,
            description: input.description,
            created_at: Utc::now(),
        };
        agents
            .insert_autonomous_agent(deployment_id, &agent)
            .await?;
        let request_id = self
            .open_request(
                &deployment,
                client_id.clone(),
                RequestKind::Create,
                input.sealing_key,
            )
            .await?;
        self.announce(&deployment, mcp_enabled).await?;

        self.write_audit(
            &identity,
            organisation_id,
            AUTONOMOUS_CREATED_AUDIT,
            deployment_id,
            AuditChange::new(
                Value::Null,
                json!({ "client_id": client_id, "name": agent.name }),
            )?,
        )
        .await?;

        Ok(request_id)
    }

    async fn rotate_autonomous_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
        sealing_key: SealingKey,
    ) -> Result<Uuid, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .ferriskey_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.mcp_enabled(organisation_id).await?;
        if !self.settings(deployment_id).await?.enabled || !mcp_enabled {
            return Err(AgentError::NotEnabled.into());
        }
        if !self
            .publisher
            .agents
            .autonomous_agents(deployment_id)
            .await?
            .iter()
            .any(|agent| agent.client_id == client_id)
        {
            return Err(AgentError::AutonomousAgentNotFound {
                client_id: client_id.to_string(),
            }
            .into());
        }

        let request_id = self
            .open_request(
                &deployment,
                client_id.clone(),
                RequestKind::Rotate,
                sealing_key,
            )
            .await?;
        self.announce(&deployment, mcp_enabled).await?;

        self.write_audit(
            &identity,
            organisation_id,
            AUTONOMOUS_ROTATED_AUDIT,
            deployment_id,
            AuditChange::new(Value::Null, json!({ "client_id": client_id }))?,
        )
        .await?;

        Ok(request_id)
    }

    async fn fetch_agent_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        request_id: Uuid,
    ) -> Result<CredentialFetch, CoreError> {
        self.policy
            .can_manage_agents(identity, organisation_id)
            .await?;
        let deployment = self
            .ferriskey_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.mcp_enabled(organisation_id).await?;

        let agents = &self.publisher.agents;
        let now: DateTime<Utc> = Utc::now();
        agents.purge_expired(now).await?;

        match agents
            .credential_state(deployment_id, request_id, now)
            .await?
        {
            CredentialState::Missing => Err(AgentError::CredentialNotFound { request_id }.into()),
            CredentialState::Delivered => Err(AgentError::CredentialGone { request_id }.into()),
            CredentialState::Pending => Ok(CredentialFetch::Pending),
            CredentialState::Ready => {
                let settings = self.settings(deployment_id).await?;
                let audience = self.mcp_url(&deployment, &settings).ok_or_else(|| {
                    AgentError::invalid("the deployment's public address is not known yet")
                })?;
                let token_url = token_url(&audience, &settings.realm).ok_or_else(|| {
                    AgentError::invalid("the deployment's public address is not known yet")
                })?;

                let taken = agents
                    .take_sealed(deployment_id, request_id, now)
                    .await?
                    .ok_or(AgentError::CredentialGone { request_id })?;
                self.announce(&deployment, mcp_enabled).await?;

                Ok(CredentialFetch::Delivered(DeliveredCredential {
                    client_id: taken.client_id,
                    sealed: taken.sealed,
                    token_url,
                    audience,
                }))
            }
        }
    }
}
