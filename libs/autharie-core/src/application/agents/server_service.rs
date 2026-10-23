use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    agents::{
        server_ports::AgentServerService,
        servers::{CustomerServer, ServerId, ServerInput},
        service::AgentServiceImpl,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use autharie_macros::transactional;

use super::InstanceRights;
use crate::{AutharieService, infrastructure::role::permissions_in};

impl AgentServerService for AutharieService {
    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn list_agent_servers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<CustomerServer>, CoreError> {
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
        .list_agent_servers(identity, organisation_id, deployment_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn register_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: ServerInput,
    ) -> Result<CustomerServer, CoreError> {
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
        .register_agent_server(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn update_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
        input: ServerInput,
    ) -> Result<CustomerServer, CoreError> {
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
        .update_agent_server(identity, organisation_id, deployment_id, server_id, input)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn remove_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
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
        .remove_agent_server(identity, organisation_id, deployment_id, server_id)
        .await
    }

    #[transactional(deployment, organisation, agent, action, audit, user)]
    async fn discover_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
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
        .discover_agent_server(identity, organisation_id, deployment_id, server_id)
        .await
    }
}
