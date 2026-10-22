use std::future::Future;

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    CoreError, agents::AgentError, deployments::DeploymentId, organisation::OrganisationId,
};

pub const MAX_DECISIONS: usize = 200;
pub const DEFAULT_DECISIONS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DecisionVerdict {
    Allow,
    Deny,
}

impl DecisionVerdict {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentDecision {
    pub kind: String,
    pub time: DateTime<Utc>,
    pub deployment_id: Uuid,
    pub server: String,
    pub resource: Option<String>,
    pub method: String,
    pub tool: Option<String>,
    pub subject_type: String,
    pub subject_id: String,
    pub agent: String,
    pub delegated: bool,
    pub decision: DecisionVerdict,
    pub policies: Vec<String>,
    pub reason: Option<String>,
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionQuery {
    pub deployment_id: DeploymentId,
    pub decision: Option<DecisionVerdict>,
    pub tool: Option<String>,
    pub subject: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: usize,
}

impl DecisionQuery {
    pub fn parse(
        decision: Option<&str>,
        tool: Option<&str>,
        subject: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: Option<usize>,
    ) -> Result<Self, AgentError> {
        let decision = decision
            .map(|value| {
                DecisionVerdict::parse(value).ok_or_else(|| {
                    AgentError::invalid(format!("'{value}' is not a decision, use allow or deny"))
                })
            })
            .transpose()?;

        if let (Some(from), Some(to)) = (from, to)
            && to <= from
        {
            return Err(AgentError::invalid("to must be after from"));
        }

        if limit == Some(0) {
            return Err(AgentError::invalid("limit must be at least 1"));
        }

        let text = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };

        Ok(Self {
            deployment_id: DeploymentId(Uuid::nil()),
            decision,
            tool: text(tool),
            subject: text(subject),
            from,
            to,
            limit: limit.unwrap_or(DEFAULT_DECISIONS).min(MAX_DECISIONS),
        })
    }
}

pub trait AgentDecisionIndex: Send + Sync {
    fn decisions(
        &self,
        organisation_id: OrganisationId,
        query: DecisionQuery,
    ) -> impl Future<Output = Result<Vec<AgentDecision>, CoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_capped_and_defaulted() {
        let capped = DecisionQuery::parse(None, None, None, None, None, Some(10_000)).unwrap();
        let defaulted = DecisionQuery::parse(None, None, None, None, None, None).unwrap();

        assert_eq!(capped.limit, MAX_DECISIONS);
        assert_eq!(defaulted.limit, DEFAULT_DECISIONS);
    }

    #[test]
    fn a_bad_verdict_a_backwards_range_and_a_zero_limit_are_refused() {
        let now = Utc::now();

        assert!(DecisionQuery::parse(Some("maybe"), None, None, None, None, None).is_err());
        assert!(DecisionQuery::parse(None, None, None, Some(now), Some(now), None).is_err());
        assert!(DecisionQuery::parse(None, None, None, None, None, Some(0)).is_err());
    }

    #[test]
    fn blank_filters_are_no_filters() {
        let query =
            DecisionQuery::parse(Some("deny"), Some(" "), Some("a"), None, None, None).unwrap();

        assert_eq!(query.decision, Some(DecisionVerdict::Deny));
        assert_eq!(query.tool, None);
        assert_eq!(query.subject.as_deref(), Some("a"));
    }
}
