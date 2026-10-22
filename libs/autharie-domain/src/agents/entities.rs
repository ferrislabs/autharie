use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::deployments::DeploymentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, format = Uuid)]
pub struct PolicyId(pub Uuid);

impl fmt::Display for PolicyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub enum AgentsPhase {
    Applying,
    Ready,
    Failed,
}

impl AgentsPhase {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "Applying" => Some(Self::Applying),
            "Ready" => Some(Self::Ready),
            "Failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Applying => "Applying",
            Self::Ready => "Ready",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub enum PolicyPhase {
    Pending,
    Applied,
    Invalid,
    Failed,
}

impl PolicyPhase {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "Pending" => Some(Self::Pending),
            "Applied" => Some(Self::Applied),
            "Invalid" => Some(Self::Invalid),
            "Failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Applied => "Applied",
            Self::Invalid => "Invalid",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentStatus {
    pub phase: AgentsPhase,
    pub gateway_client_ready: bool,
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSettings {
    pub deployment_id: DeploymentId,
    pub enabled: bool,
    pub realm: String,
    pub revision: u64,
    pub applied_revision: u64,
    pub mcp_url: Option<String>,
    pub status: Option<AgentStatus>,
}

impl AgentSettings {
    pub fn unset(deployment_id: DeploymentId) -> Self {
        Self {
            deployment_id,
            enabled: false,
            realm: super::DEFAULT_REALM.to_string(),
            revision: 0,
            applied_revision: 0,
            mcp_url: None,
            status: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentsView {
    pub enabled: bool,
    pub realm: String,
    pub mcp_url: Option<String>,
    pub organisation_mcp_enabled: bool,
    pub status: Option<AgentStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentPolicy {
    pub id: PolicyId,
    #[serde(skip)]
    pub deployment_id: DeploymentId,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    #[schema(value_type = Option<Object>)]
    pub rule: Option<Value>,
    pub cedar: String,
    pub phase: Option<PolicyPhase>,
    pub message: Option<String>,
    #[serde(skip)]
    pub desired_revision: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
