use autharie_auth::Identity;
use chrono::Utc;
use serde_json::{Value, json};

use crate::{
    CoreError,
    action::{
        ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    agents::{
        AGENT_POLICY_CREATED_AUDIT, AGENT_POLICY_DELETED_AUDIT, AGENT_POLICY_UPDATED_AUDIT,
        AGENTS_ACTION_TYPE, AGENTS_UPDATED_AUDIT, AgentError, MAX_POLICIES,
        commands::{AgentSettingsInput, AgentsOutcome, PolicyInput},
        decision::{AgentDecision, AgentDecisionIndex, DecisionQuery},
        entities::{AgentPolicy, AgentSettings, AgentStatus, AgentsView, PolicyId, PolicyPhase},
        ports::{AgentAccess, AgentOutcomeService, AgentRepository, AgentService},
        registry::{MAX_REGISTRY_CLIENTS, RequestKind},
        registry_ports::AgentRegistryRepository,
        server_ports::AgentServerRepository,
    },
    audit::{
        AuditAction, AuditChange, AuditEntry, AuditEntryId, AuditTarget, AuditTargetKind,
        ports::AuditRepository, service::audit_actor,
    },
    dataplane::{herald_identity::speaking_for, ports::DataPlaneRepository},
    deployments::{Deployment, DeploymentId, DeploymentKind, ports::DeploymentRepository},
    generate_uuid_v7,
    organisation::{OrganisationId, ports::OrganisationRepository},
    user::ports::UserRepository,
};

mod servers;

pub struct AgentPublisher<D, G, A>
where
    A: ActionRepository,
{
    pub(super) deployments: D,
    pub(super) agents: G,
    actions: ActionServiceImpl<A>,
}

impl<D, G, A> AgentPublisher<D, G, A>
where
    D: DeploymentRepository,
    G: AgentRepository + AgentRegistryRepository + AgentServerRepository,
    A: ActionRepository,
{
    pub fn new(deployments: D, agents: G, actions: A) -> Self {
        Self {
            deployments,
            agents,
            actions: ActionServiceImpl::new(actions),
        }
    }

    pub async fn publish(
        &self,
        deployment: &Deployment,
        organisation_mcp_enabled: bool,
    ) -> Result<(), CoreError> {
        let settings = self
            .agents
            .settings(deployment.id)
            .await?
            .unwrap_or_else(|| AgentSettings::unset(deployment.id));
        let policies: Vec<Value> = self
            .agents
            .list_policies(deployment.id)
            .await?
            .into_iter()
            .filter(|policy| policy.enabled)
            .map(|policy| {
                json!({
                    "id": policy.id.0,
                    "name": policy.name,
                    "cedar": policy.cedar,
                })
            })
            .collect();
        let servers: Vec<Value> = self
            .agents
            .list_servers(deployment.id)
            .await?
            .into_iter()
            .map(|server| {
                json!({
                    "id": server.id.0,
                    "slug": server.slug,
                    "url": server.url,
                    "enabled": server.enabled,
                    "discovery_requested_at": server.discovery_requested_at,
                })
            })
            .collect();

        let access = self.agents.access(deployment.id).await?;
        let requests = self.agents.open_requests(deployment.id, Utc::now()).await?;
        let autonomous: Vec<Value> = self
            .agents
            .autonomous_agents(deployment.id)
            .await?
            .into_iter()
            .map(|agent| {
                let mut entry = json!({
                    "client_id": agent.client_id,
                    "name": agent.name,
                });
                if let Some(request) = requests
                    .iter()
                    .filter(|request| request.client_id == agent.client_id)
                    .max_by_key(|request| request.expires_at)
                {
                    let key = match request.kind {
                        RequestKind::Create => "request_id",
                        RequestKind::Rotate => "rotate_request_id",
                    };
                    entry[key] = json!(request.request_id);
                    entry["sealing_key"] = request.sealing_key.payload();
                }
                entry
            })
            .collect();
        let deleted: Vec<&String> = access.deleted.iter().collect();

        let data = json!({
            "deployment_id": deployment.id.0,
            "namespace": deployment.namespace,
            "enabled": settings.enabled && organisation_mcp_enabled,
            "realm": settings.realm,
            "revision": settings.revision,
            "policies": policies,
            "access": access.payload(),
            "autonomous": autonomous,
            "deleted_agents": deleted,
            "servers": servers,
        });

        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(AGENTS_ACTION_TYPE.to_string()),
                ActionTarget {
                    kind: TargetKind::Deployment,
                    id: deployment.id.0,
                },
                ActionPayload { data },
                ActionVersion(1),
                ActionSource::System,
            ))
            .await?;

        Ok(())
    }

    pub async fn republish_organisation(
        &self,
        organisation_id: OrganisationId,
        organisation_mcp_enabled: bool,
    ) -> Result<usize, CoreError> {
        let mut sent = 0;

        for deployment in self
            .deployments
            .list_by_organisation(organisation_id)
            .await?
        {
            if deployment.kind != DeploymentKind::Ferriskey || deployment.deleted_at.is_some() {
                continue;
            }
            if self.agents.settings(deployment.id).await?.is_none() {
                continue;
            }

            self.agents.next_revision(deployment.id, Utc::now()).await?;
            self.publish(&deployment, organisation_mcp_enabled).await?;
            sent += 1;
        }

        Ok(sent)
    }
}

fn policy_summary(policy: &AgentPolicy) -> Value {
    json!({
        "deployment_id": policy.deployment_id.0,
        "name": policy.name,
        "description": policy.description,
        "enabled": policy.enabled,
        "cedar": policy.cedar,
    })
}

pub struct AgentServiceImpl<D, O, G, A, AU, U, P>
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

impl<D, O, G, A, AU, U, P> AgentServiceImpl<D, O, G, A, AU, U, P>
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
        G: AgentRepository + AgentRegistryRepository + AgentServerRepository,
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

impl<D, O, G, A, AU, U, P> AgentServiceImpl<D, O, G, A, AU, U, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    G: AgentRepository + AgentRegistryRepository + AgentServerRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: AgentAccess,
{
    async fn deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        self.publisher
            .deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .filter(|deployment| deployment.deleted_at.is_none())
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })
    }

    async fn managed_deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        let deployment = self.deployment(organisation_id, deployment_id).await?;

        if deployment.kind != DeploymentKind::Ferriskey {
            return Err(AgentError::UnsupportedKind.into());
        }

        Ok(deployment)
    }

    async fn organisation_mcp_enabled(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<bool, CoreError> {
        Ok(self
            .organisations
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })?
            .mcp_enabled)
    }

    async fn stored_policy(
        &self,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
    ) -> Result<AgentPolicy, CoreError> {
        self.publisher
            .agents
            .get_policy(deployment_id, policy_id)
            .await?
            .ok_or_else(|| AgentError::PolicyNotFound { id: policy_id.0 }.into())
    }

    /// The data plane reports the URL its gateway actually serves, port
    /// included; the deployment's host is only a guess until it has.
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

    async fn view(
        &self,
        deployment: &Deployment,
        settings: AgentSettings,
    ) -> Result<AgentsView, CoreError> {
        Ok(AgentsView {
            enabled: settings.enabled,
            mcp_url: self.mcp_url(deployment, &settings),
            organisation_mcp_enabled: self
                .organisation_mcp_enabled(deployment.organisation_id)
                .await?,
            realm: settings.realm,
            status: settings.status,
        })
    }

    async fn write_audit(
        &self,
        identity: &Identity,
        organisation_id: OrganisationId,
        action: &str,
        kind: &str,
        id: uuid::Uuid,
        change: AuditChange,
    ) -> Result<(), CoreError> {
        self.audit
            .append(AuditEntry::record(
                AuditEntryId(generate_uuid_v7()),
                organisation_id,
                audit_actor(identity, &self.users).await?,
                AuditAction(action.to_string()),
                AuditTarget {
                    kind: AuditTargetKind::Custom(kind.to_string()),
                    id,
                },
                Some(change),
                Utc::now(),
            ))
            .await
    }
}

impl<D, O, G, A, AU, U, P> AgentService for AgentServiceImpl<D, O, G, A, AU, U, P>
where
    D: DeploymentRepository,
    O: OrganisationRepository,
    G: AgentRepository + AgentRegistryRepository + AgentServerRepository,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: AgentAccess,
{
    async fn get_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<AgentsView, CoreError> {
        self.policy
            .can_view_agents(identity, organisation_id)
            .await?;
        let deployment = self.deployment(organisation_id, deployment_id).await?;

        let settings = self
            .publisher
            .agents
            .settings(deployment_id)
            .await?
            .unwrap_or_else(|| AgentSettings::unset(deployment_id));

        self.view(&deployment, settings).await
    }

    async fn set_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: AgentSettingsInput,
    ) -> Result<AgentsView, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;

        let before = self
            .publisher
            .agents
            .settings(deployment_id)
            .await?
            .unwrap_or_else(|| AgentSettings::unset(deployment_id));
        let settings = self
            .publisher
            .agents
            .save_settings(deployment_id, &input, Utc::now())
            .await?;
        self.publisher.publish(&deployment, mcp_enabled).await?;

        self.write_audit(
            &identity,
            organisation_id,
            AGENTS_UPDATED_AUDIT,
            "deployment_agents",
            deployment_id.0,
            AuditChange::new(
                json!({ "enabled": before.enabled, "realm": before.realm }),
                json!({ "enabled": settings.enabled, "realm": settings.realm }),
            )?,
        )
        .await?;

        self.view(&deployment, settings).await
    }

    async fn list_agent_policies(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<AgentPolicy>, CoreError> {
        self.policy
            .can_view_agents(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        self.publisher.agents.list_policies(deployment_id).await
    }

    async fn create_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: PolicyInput,
    ) -> Result<AgentPolicy, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;

        if self
            .publisher
            .agents
            .list_policies(deployment_id)
            .await?
            .len()
            >= MAX_POLICIES
        {
            return Err(AgentError::TooManyPolicies { max: MAX_POLICIES }.into());
        }

        let now = Utc::now();
        let revision = self
            .publisher
            .agents
            .next_revision(deployment_id, now)
            .await?;
        let policy = AgentPolicy {
            id: PolicyId(generate_uuid_v7()),
            deployment_id,
            name: input.name,
            description: input.description,
            enabled: input.enabled,
            rule: input.rule,
            cedar: input.cedar,
            phase: input.enabled.then_some(PolicyPhase::Pending),
            message: None,
            desired_revision: revision,
            created_at: now,
            updated_at: now,
        };

        self.publisher.agents.insert_policy(&policy).await?;
        self.publisher.publish(&deployment, mcp_enabled).await?;
        self.write_audit(
            &identity,
            organisation_id,
            AGENT_POLICY_CREATED_AUDIT,
            "agent_policy",
            policy.id.0,
            AuditChange::new(Value::Null, policy_summary(&policy))?,
        )
        .await?;

        Ok(policy)
    }

    async fn update_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
        input: PolicyInput,
    ) -> Result<AgentPolicy, CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;
        let before = self.stored_policy(deployment_id, policy_id).await?;

        let now = Utc::now();
        let revision = self
            .publisher
            .agents
            .next_revision(deployment_id, now)
            .await?;
        let policy = AgentPolicy {
            name: input.name,
            description: input.description,
            enabled: input.enabled,
            rule: input.rule,
            cedar: input.cedar,
            phase: input.enabled.then_some(PolicyPhase::Pending),
            message: None,
            desired_revision: revision,
            updated_at: now,
            ..before.clone()
        };

        if !self.publisher.agents.update_policy(&policy).await? {
            return Err(AgentError::PolicyNotFound { id: policy_id.0 }.into());
        }
        self.publisher.publish(&deployment, mcp_enabled).await?;
        self.write_audit(
            &identity,
            organisation_id,
            AGENT_POLICY_UPDATED_AUDIT,
            "agent_policy",
            policy_id.0,
            AuditChange::new(policy_summary(&before), policy_summary(&policy))?,
        )
        .await?;

        Ok(policy)
    }

    async fn delete_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_agents(identity.clone(), organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        let mcp_enabled = self.organisation_mcp_enabled(organisation_id).await?;
        let before = self.stored_policy(deployment_id, policy_id).await?;

        self.publisher
            .agents
            .next_revision(deployment_id, Utc::now())
            .await?;
        if !self
            .publisher
            .agents
            .delete_policy(deployment_id, policy_id)
            .await?
        {
            return Err(AgentError::PolicyNotFound { id: policy_id.0 }.into());
        }
        self.publisher.publish(&deployment, mcp_enabled).await?;

        self.write_audit(
            &identity,
            organisation_id,
            AGENT_POLICY_DELETED_AUDIT,
            "agent_policy",
            policy_id.0,
            AuditChange::new(policy_summary(&before), Value::Null)?,
        )
        .await
    }

    async fn agent_decisions<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        mut query: DecisionQuery,
        index: I,
    ) -> Result<Vec<AgentDecision>, CoreError>
    where
        I: AgentDecisionIndex,
    {
        self.policy
            .can_view_agents(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        query.deployment_id = deployment_id;

        index.decisions(organisation_id, query).await
    }
}

pub struct AgentOutcomeServiceImpl<D, P, G> {
    deployments: D,
    dataplanes: P,
    agents: G,
}

impl<D, P, G> AgentOutcomeServiceImpl<D, P, G> {
    pub fn new(deployments: D, dataplanes: P, agents: G) -> Self {
        Self {
            deployments,
            dataplanes,
            agents,
        }
    }
}

impl<D, P, G> AgentOutcomeService for AgentOutcomeServiceImpl<D, P, G>
where
    D: DeploymentRepository,
    P: DataPlaneRepository,
    G: AgentRepository + AgentRegistryRepository + AgentServerRepository,
{
    async fn report_agents_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: AgentsOutcome,
    ) -> Result<bool, CoreError> {
        let speaking = speaking_for(&self.dataplanes, &identity).await?;

        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;

        let Some(settings) = self.agents.settings(deployment_id).await? else {
            return Ok(false);
        };
        if outcome.revision < settings.applied_revision || outcome.revision > settings.revision {
            return Ok(false);
        }

        let now = Utc::now();
        let status = AgentStatus {
            phase: outcome.phase,
            gateway_client_ready: outcome.gateway_client_ready,
            message: outcome.message.clone(),
            updated_at: now,
        };

        let mut changed = self
            .agents
            .record_status(
                deployment_id,
                outcome.revision,
                &status,
                outcome.mcp_url.as_deref(),
            )
            .await?;

        for report in &outcome.policies {
            changed |= self
                .agents
                .record_policy_outcome(
                    deployment_id,
                    report.id,
                    outcome.revision,
                    report.phase,
                    report.message.as_deref(),
                    now,
                )
                .await?;
        }

        for report in &outcome.servers {
            if let Some(discovery) = &report.discovery {
                changed |= self
                    .agents
                    .record_discovery(deployment_id, report.id, discovery, report.answers)
                    .await?;
            }
        }
        self.agents.purge_expired(now).await?;
        self.ingest_registry(deployment_id, &outcome, now).await?;

        Ok(changed)
    }
}

impl<D, P, G> AgentOutcomeServiceImpl<D, P, G>
where
    G: AgentRepository + AgentRegistryRepository,
{
    async fn ingest_registry(
        &self,
        deployment_id: DeploymentId,
        outcome: &AgentsOutcome,
        now: chrono::DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let mut registry = outcome.registry.clone();
        if registry.clients.len() > MAX_REGISTRY_CLIENTS {
            registry.clients.truncate(MAX_REGISTRY_CLIENTS);
            registry.truncated = true;
        }
        self.agents
            .replace_registry(deployment_id, &registry, now)
            .await?;

        if !registry.truncated {
            let mut access = self.agents.access(deployment_id).await?;
            let before = access.deleted.len();
            access.deleted.retain(|gone| {
                registry
                    .clients
                    .iter()
                    .any(|client| client.client_id.as_str() == gone)
            });
            if access.deleted.len() != before {
                self.agents.save_access(deployment_id, &access, now).await?;
            }
        }

        for credential in &outcome.credentials {
            self.agents
                .store_sealed(
                    deployment_id,
                    credential.request_id,
                    &credential.client_id,
                    &credential.sealed,
                    now,
                )
                .await?;
        }

        Ok(())
    }
}
