use thiserror::Error;
use uuid::Uuid;

pub mod commands;
pub mod decision;
pub mod entities;
pub mod ports;
pub mod registry;
pub mod registry_ports;
pub mod registry_service;
#[cfg(test)]
mod registry_tests;
pub mod service;
#[cfg(test)]
mod service_tests;

pub use commands::{AgentSettingsInput, AgentsOutcome, PolicyInput, PolicyOutcome};
pub use decision::{AgentDecision, DecisionQuery, DecisionVerdict, MAX_DECISIONS};
pub use entities::{
    AgentPolicy, AgentSettings, AgentStatus, AgentsPhase, AgentsView, PolicyId, PolicyPhase,
};
pub use registry::{
    AccessLists, AccessState, AgentClient, AgentClientId, AutonomousInput, ClientKind,
    ClientSource, CredentialFetch, DeliveredCredential, RegistryView, SealingKey,
};

pub const AGENTS_ACTION_TYPE: &str = "deployment.agents";

pub const AGENTS_UPDATED_AUDIT: &str = "deployment.agents.updated";
pub const AGENT_POLICY_CREATED_AUDIT: &str = "deployment.agent_policy.created";
pub const AGENT_POLICY_UPDATED_AUDIT: &str = "deployment.agent_policy.updated";
pub const AGENT_POLICY_DELETED_AUDIT: &str = "deployment.agent_policy.deleted";

pub const DEFAULT_REALM: &str = "master";
pub const MAX_POLICIES: usize = 100;
pub const MAX_AUTONOMOUS_AGENTS_PER_DEPLOYMENT: usize = 100;
pub const MAX_CEDAR_BYTES: usize = 64 * 1024;
pub const MAX_RULE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentError {
    #[error("{reason}")]
    Invalid { reason: String },

    #[error("the Cedar policy does not parse: {message}")]
    InvalidCedar { message: String },

    #[error("agent policy {id} not found")]
    PolicyNotFound { id: Uuid },

    #[error("agents are only available on FerrisKey deployments")]
    UnsupportedKind,

    #[error("a deployment holds at most {max} agent policies")]
    TooManyPolicies { max: usize },

    #[error("the sealing key is refused: {reason}")]
    InvalidSealingKey { reason: String },

    #[error("agents are not enabled on this deployment")]
    NotEnabled,

    #[error("the deployment's public address is not known yet")]
    AddressUnknown,

    #[error("no autonomous agent {client_id} on this deployment")]
    AutonomousAgentNotFound { client_id: String },

    #[error("a deployment holds at most {max} autonomous agents")]
    TooManyAutonomousAgents { max: usize },

    #[error("no credential request {request_id} on this deployment, or it expired")]
    CredentialNotFound { request_id: Uuid },

    #[error("the credential of request {request_id} was already delivered")]
    CredentialGone { request_id: Uuid },
}

impl AgentError {
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }
}
