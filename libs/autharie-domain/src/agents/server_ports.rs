use std::future::Future;

use autharie_auth::Identity;

use crate::{
    CoreError,
    agents::servers::{CustomerServer, Discovery, ServerId, ServerInput},
    deployments::DeploymentId,
    organisation::OrganisationId,
};

pub trait AgentServerRepository: Send + Sync {
    fn list_servers(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<CustomerServer>, CoreError>> + Send;

    fn get_server(
        &self,
        deployment_id: DeploymentId,
        id: ServerId,
    ) -> impl Future<Output = Result<Option<CustomerServer>, CoreError>> + Send;

    fn insert_server(
        &self,
        server: &CustomerServer,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn update_server(
        &self,
        server: &CustomerServer,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn delete_server(
        &self,
        deployment_id: DeploymentId,
        id: ServerId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn record_discovery(
        &self,
        deployment_id: DeploymentId,
        id: ServerId,
        discovery: &Discovery,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

pub trait AgentServerService: Send + Sync {
    fn list_agent_servers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<CustomerServer>, CoreError>> + Send;

    fn register_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: ServerInput,
    ) -> impl Future<Output = Result<CustomerServer, CoreError>> + Send;

    fn update_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
        input: ServerInput,
    ) -> impl Future<Output = Result<CustomerServer, CoreError>> + Send;

    fn remove_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn discover_agent_server(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        server_id: ServerId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}
