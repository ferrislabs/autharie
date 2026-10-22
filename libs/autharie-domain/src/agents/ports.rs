use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    agents::{
        commands::{AgentSettingsInput, AgentsOutcome, PolicyInput},
        decision::{AgentDecision, AgentDecisionIndex, DecisionQuery},
        entities::{AgentPolicy, AgentSettings, AgentStatus, AgentsView, PolicyId, PolicyPhase},
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};

pub trait AgentAccess: Send + Sync {
    fn can_view_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn can_manage_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait AgentRepository: Send + Sync {
    fn settings(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Option<AgentSettings>, CoreError>> + Send;

    fn save_settings(
        &self,
        deployment_id: DeploymentId,
        input: &AgentSettingsInput,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<AgentSettings, CoreError>> + Send;

    fn next_revision(
        &self,
        deployment_id: DeploymentId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;

    fn record_status(
        &self,
        deployment_id: DeploymentId,
        revision: u64,
        status: &AgentStatus,
        mcp_url: Option<&str>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn list_policies(
        &self,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<AgentPolicy>, CoreError>> + Send;

    fn get_policy(
        &self,
        deployment_id: DeploymentId,
        id: PolicyId,
    ) -> impl Future<Output = Result<Option<AgentPolicy>, CoreError>> + Send;

    fn insert_policy(
        &self,
        policy: &AgentPolicy,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn update_policy(
        &self,
        policy: &AgentPolicy,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn delete_policy(
        &self,
        deployment_id: DeploymentId,
        id: PolicyId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn record_policy_outcome(
        &self,
        deployment_id: DeploymentId,
        id: PolicyId,
        revision: u64,
        phase: PolicyPhase,
        message: Option<&str>,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

pub trait AgentService: Send + Sync {
    fn get_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<AgentsView, CoreError>> + Send;

    fn set_agents(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: AgentSettingsInput,
    ) -> impl Future<Output = Result<AgentsView, CoreError>> + Send;

    fn list_agent_policies(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<AgentPolicy>, CoreError>> + Send;

    fn create_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: PolicyInput,
    ) -> impl Future<Output = Result<AgentPolicy, CoreError>> + Send;

    fn update_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
        input: PolicyInput,
    ) -> impl Future<Output = Result<AgentPolicy, CoreError>> + Send;

    fn delete_agent_policy(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        policy_id: PolicyId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn agent_decisions<I>(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        query: DecisionQuery,
        index: I,
    ) -> impl Future<Output = Result<Vec<AgentDecision>, CoreError>> + Send
    where
        I: AgentDecisionIndex;
}

pub trait AgentOutcomeService: Send + Sync {
    fn report_agents_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: AgentsOutcome,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
