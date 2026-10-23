use autharie_auth::Identity;
use autharie_domain::agents::{
    AgentSettingsInput, AgentsOutcome, PolicyInput,
    decision::AgentDecisionIndex,
    ports::{AgentAccess, AgentOutcomeService, AgentService},
    service::{AgentOutcomeServiceImpl, AgentServiceImpl},
};
use autharie_domain::{
    CoreError, deployments::DeploymentId, organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;

pub use autharie_domain::agents::{
    AGENTS_ACTION_TYPE, AgentDecision, AgentError, AgentPolicy, AgentSettings, AgentStatus,
    AgentsPhase, AgentsView, DEFAULT_REALM, DecisionQuery, DecisionVerdict, MAX_DECISIONS,
    PolicyId, PolicyPhase, commands, decision, entities, ports, registry, registry_ports,
    registry_service, service,
};

use crate::{AutharieService, infrastructure::role::permissions_in, policy::PolicyContext};

mod registry_impl;

struct InstanceRights<R> {
    permissions: R,
}

impl<R> AgentAccess for InstanceRights<R>
where
    R: PermissionProvider,
{
    async fn can_view_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        PolicyContext::new(permissions).require_permission(Permissions::VIEW_INSTANCES)
    }

    async fn can_manage_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        PolicyContext::new(permissions).require_permission(Permissions::MANAGE_INSTANCES)
    }
}

impl AgentService for AutharieService {
    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn get_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<AgentsView, CoreError> {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            self.deployment_domain().map(str::to_owned),
        )
        .get_agents(identity, organisation_id, deployment_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn set_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: AgentSettingsInput,
    ) -> Result<AgentsView, CoreError> {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            self.deployment_domain().map(str::to_owned),
        )
        .set_agents(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn list_agent_policies(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<AgentPolicy>, CoreError> {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            None,
        )
        .list_agent_policies(identity, organisation_id, deployment_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn create_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: PolicyInput,
    ) -> Result<AgentPolicy, CoreError> {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            None,
        )
        .create_agent_policy(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn update_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
        input: PolicyInput,
    ) -> Result<AgentPolicy, CoreError> {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            None,
        )
        .update_agent_policy(identity, organisation_id, deployment_id, policy_id, input)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn delete_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
    ) -> Result<(), CoreError> {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            None,
        )
        .delete_agent_policy(identity, organisation_id, deployment_id, policy_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn agent_decisions<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        query: DecisionQuery,
        index: I,
    ) -> Result<Vec<AgentDecision>, CoreError>
    where
        I: AgentDecisionIndex,
    {
        AgentServiceImpl::new(
            deployment_repository,
            organisation_repository,
            agent_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
            None,
        )
        .agent_decisions(identity, organisation_id, deployment_id, query, index)
        .await
    }
}

impl AgentOutcomeService for AutharieService {
    #[transactional(data_plane, deployment, agent)]
    async fn report_agents_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: AgentsOutcome,
    ) -> Result<bool, CoreError> {
        AgentOutcomeServiceImpl::new(
            deployment_repository,
            data_plane_repository,
            agent_repository,
        )
        .report_agents_outcome(identity, deployment_id, outcome)
        .await
    }
}
