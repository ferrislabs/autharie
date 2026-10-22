use thiserror::Error;
use uuid::Uuid;

pub mod commands;
pub mod decision;
pub mod entities;
pub mod ports;
pub mod service;
#[cfg(test)]
mod service_tests;

pub use commands::{AgentSettingsInput, AgentsOutcome, PolicyInput, PolicyOutcome};
pub use decision::{AgentDecision, DecisionQuery, DecisionVerdict, MAX_DECISIONS};
pub use entities::{
    AgentPolicy, AgentSettings, AgentStatus, AgentsPhase, AgentsView, PolicyId, PolicyPhase,
};

pub const AGENTS_ACTION_TYPE: &str = "deployment.agents";

pub const AGENTS_UPDATED_AUDIT: &str = "deployment.agents.updated";
pub const AGENT_POLICY_CREATED_AUDIT: &str = "deployment.agent_policy.created";
pub const AGENT_POLICY_UPDATED_AUDIT: &str = "deployment.agent_policy.updated";
pub const AGENT_POLICY_DELETED_AUDIT: &str = "deployment.agent_policy.deleted";

pub const DEFAULT_REALM: &str = "master";
pub const MAX_POLICIES: usize = 100;
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
}

impl AgentError {
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }
}
