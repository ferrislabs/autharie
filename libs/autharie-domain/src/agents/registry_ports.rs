use std::{collections::BTreeMap, future::Future};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    CoreError,
    agents::registry::{
        AccessLists, AgentClientId, AutonomousAgent, AutonomousInput, CredentialFetch,
        CredentialRequest, CredentialState, RegistryView, ReportedRegistry, SealedCredential,
        SealingKey,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};

pub trait AgentRegistryRepository: Send + Sync {
    fn access(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<AccessLists, CoreError>> + Send;

    fn save_access(
        &self,
        deployment_id: DeploymentId,
        access: &AccessLists,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn registry(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<ReportedRegistry, CoreError>> + Send;

    fn replace_registry(
        &self,
        deployment_id: DeploymentId,
        registry: &ReportedRegistry,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn autonomous_agents(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<AutonomousAgent>, CoreError>> + Send;

    fn insert_autonomous_agent(
        &self,
        deployment_id: DeploymentId,
        agent: &AutonomousAgent,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn delete_autonomous_agent(
        &self,
        deployment_id: DeploymentId,
        client_id: &AgentClientId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn insert_request(
        &self,
        deployment_id: DeploymentId,
        request: &CredentialRequest,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn open_requests(
        &self,
        deployment_id: DeploymentId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<CredentialRequest>, CoreError>> + Send;

    fn store_sealed(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        client_id: &AgentClientId,
        sealed: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn credential_state(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<CredentialState, CoreError>> + Send;

    fn take_sealed(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<SealedCredential>, CoreError>> + Send;

    fn purge_expired(
        &self,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;
}

pub trait AgentActivityIndex: Send + Sync {
    fn last_seen(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        since: DateTime<Utc>,
    ) -> impl Future<Output = Result<BTreeMap<String, DateTime<Utc>>, CoreError>> + Send;
}

pub trait AgentRegistryService: Send + Sync {
    fn list_agent_clients<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        activity: I,
    ) -> impl Future<Output = Result<RegistryView, CoreError>> + Send
    where
        I: AgentActivityIndex;

    fn set_approval_mode(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        approval_required: bool,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn approve_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn block_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn unblock_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn delete_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn create_autonomous_agent(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: AutonomousInput,
    ) -> impl Future<Output = Result<Uuid, CoreError>> + Send;

    fn rotate_autonomous_secret(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        client_id: AgentClientId,
        sealing_key: SealingKey,
    ) -> impl Future<Output = Result<Uuid, CoreError>> + Send;

    fn fetch_agent_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        request_id: Uuid,
    ) -> impl Future<Output = Result<CredentialFetch, CoreError>> + Send;
}

impl<I> AgentActivityIndex for Option<I>
where
    I: AgentActivityIndex,
{
    async fn last_seen(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        since: DateTime<Utc>,
    ) -> Result<BTreeMap<String, DateTime<Utc>>, CoreError> {
        match self {
            Some(index) => index.last_seen(organisation_id, deployment_id, since).await,
            None => Ok(BTreeMap::new()),
        }
    }
}
