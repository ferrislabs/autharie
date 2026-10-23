use autharie_auth::Identity;
use autharie_domain::agents::{
    registry::{AgentClientId, AutonomousInput, CredentialFetch, RegistryView, SealingKey},
    registry_ports::AgentRegistryRepository,
    registry_ports::{AgentActivityIndex, AgentRegistryService},
    registry_service::AgentRegistryServiceImpl,
};
use autharie_domain::{CoreError, deployments::DeploymentId, organisation::OrganisationId};
use autharie_macros::transactional;
use uuid::Uuid;

use super::InstanceRights;
use crate::{AutharieService, infrastructure::role::permissions_in};

impl AutharieService {
    #[transactional(agent)]
    pub async fn purge_expired_agent_credentials(&self) -> Result<u64, CoreError> {
        agent_repository.purge_expired(chrono::Utc::now()).await
    }
}

impl AgentRegistryService for AutharieService {
    #[transactional(deployment, organisation, agent, action, audit, user)]
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
        AgentRegistryServiceImpl::new(
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
        .list_agent_clients(identity, organisation_id, deployment_id, activity)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn set_approval_mode(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        approval_required: bool,
    ) -> Result<(), CoreError> {
        AgentRegistryServiceImpl::new(
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
        .set_approval_mode(identity, organisation_id, deployment_id, approval_required)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn approve_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        AgentRegistryServiceImpl::new(
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
        .approve_agent(identity, organisation_id, deployment_id, client_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn block_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        AgentRegistryServiceImpl::new(
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
        .block_agent(identity, organisation_id, deployment_id, client_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn unblock_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        AgentRegistryServiceImpl::new(
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
        .unblock_agent(identity, organisation_id, deployment_id, client_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn delete_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> Result<(), CoreError> {
        AgentRegistryServiceImpl::new(
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
        .delete_agent(identity, organisation_id, deployment_id, client_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn create_autonomous_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: AutonomousInput,
    ) -> Result<Uuid, CoreError> {
        AgentRegistryServiceImpl::new(
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
        .create_autonomous_agent(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn rotate_autonomous_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
        sealing_key: SealingKey,
    ) -> Result<Uuid, CoreError> {
        AgentRegistryServiceImpl::new(
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
        .rotate_autonomous_secret(
            identity,
            organisation_id,
            deployment_id,
            client_id,
            sealing_key,
        )
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn fetch_agent_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        request_id: Uuid,
    ) -> Result<CredentialFetch, CoreError> {
        AgentRegistryServiceImpl::new(
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
        .fetch_agent_credential(identity, organisation_id, deployment_id, request_id)
        .await
    }
}
