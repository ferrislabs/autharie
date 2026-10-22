use std::str::FromStr;

use cedar_policy::PolicySet;
use serde_json::Value;
use uuid::Uuid;

use crate::agents::{
    AgentError, DEFAULT_REALM, MAX_CEDAR_BYTES, MAX_RULE_BYTES,
    entities::{AgentsPhase, PolicyId, PolicyPhase},
};

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_REALM_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSettingsInput {
    pub enabled: bool,
    pub realm: String,
}

impl AgentSettingsInput {
    pub fn parse(enabled: bool, realm: Option<&str>) -> Result<Self, AgentError> {
        let realm = realm.map_or(DEFAULT_REALM, str::trim);

        let well_formed = !realm.is_empty()
            && realm.chars().count() <= MAX_REALM_CHARS
            && realm != "."
            && realm != ".."
            && realm
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !well_formed {
            return Err(AgentError::invalid("the realm is not a valid realm name"));
        }

        Ok(Self {
            enabled,
            realm: realm.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyInput {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub rule: Option<Value>,
    pub cedar: String,
}

impl PolicyInput {
    pub fn parse(
        name: &str,
        description: Option<&str>,
        enabled: bool,
        rule: Option<Value>,
        cedar: &str,
    ) -> Result<Self, AgentError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(AgentError::invalid(format!(
                "the name must be 1 to {MAX_NAME_CHARS} characters"
            )));
        }

        let description = description.map(str::trim).filter(|text| !text.is_empty());
        if description.is_some_and(|text| text.chars().count() > MAX_DESCRIPTION_CHARS) {
            return Err(AgentError::invalid(format!(
                "the description is at most {MAX_DESCRIPTION_CHARS} characters"
            )));
        }

        if let Some(rule) = &rule
            && rule.to_string().len() > MAX_RULE_BYTES
        {
            return Err(AgentError::invalid("the rule is too large"));
        }

        Ok(Self {
            name: name.to_string(),
            description: description.map(str::to_string),
            enabled,
            rule: rule.filter(|rule| !rule.is_null()),
            cedar: check_cedar(cedar)?,
        })
    }
}

pub fn check_cedar(source: &str) -> Result<String, AgentError> {
    if source.trim().is_empty() {
        return Err(AgentError::invalid("the Cedar policy is empty"));
    }
    if source.len() > MAX_CEDAR_BYTES {
        return Err(AgentError::invalid("the Cedar policy is too large"));
    }

    let set = PolicySet::from_str(source).map_err(|error| AgentError::InvalidCedar {
        message: error.to_string(),
    })?;
    if set.policies().next().is_none() {
        return Err(AgentError::InvalidCedar {
            message: "the source holds no policy".to_string(),
        });
    }

    Ok(source.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyOutcome {
    pub id: PolicyId,
    pub phase: PolicyPhase,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsOutcome {
    pub revision: u64,
    pub phase: AgentsPhase,
    pub gateway_client_ready: bool,
    pub mcp_url: Option<String>,
    pub message: Option<String>,
    pub policies: Vec<PolicyOutcome>,
}

impl AgentsOutcome {
    #[allow(clippy::too_many_arguments)]
    pub fn parse(
        revision: u64,
        phase: &str,
        gateway_client_ready: bool,
        mcp_url: Option<String>,
        message: Option<String>,
        policies: Vec<(Uuid, String, Option<String>)>,
    ) -> Result<Self, AgentError> {
        let phase = AgentsPhase::parse(phase)
            .ok_or_else(|| AgentError::invalid(format!("'{phase}' is not an agents phase")))?;

        let policies = policies
            .into_iter()
            .map(|(id, phase, message)| {
                let phase = PolicyPhase::parse(&phase)
                    .filter(|phase| *phase != PolicyPhase::Pending)
                    .ok_or_else(|| {
                        AgentError::invalid(format!("'{phase}' is not a policy outcome"))
                    })?;

                Ok(PolicyOutcome {
                    id: PolicyId(id),
                    phase,
                    message,
                })
            })
            .collect::<Result<_, AgentError>>()?;

        Ok(Self {
            revision,
            phase,
            gateway_client_ready,
            mcp_url,
            message,
            policies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERMIT: &str = "permit(principal, action, resource);";

    #[test]
    fn a_policy_that_parses_is_kept_as_written() {
        let input = PolicyInput::parse(" read-only ", None, true, None, PERMIT).expect("accepted");

        assert_eq!(input.name, "read-only");
        assert_eq!(input.cedar, PERMIT);
    }

    #[test]
    fn a_cedar_syntax_error_is_reported_with_its_message() {
        let error =
            PolicyInput::parse("p", None, true, None, "permit(principal,").expect_err("refused");

        assert!(matches!(error, AgentError::InvalidCedar { ref message } if !message.is_empty()));
    }

    #[test]
    fn empty_cedar_and_a_comment_only_source_are_refused() {
        assert!(PolicyInput::parse("p", None, true, None, "  ").is_err());
        assert!(matches!(
            PolicyInput::parse("p", None, true, None, "// nothing here"),
            Err(AgentError::InvalidCedar { .. })
        ));
    }

    #[test]
    fn a_blank_or_long_name_is_refused() {
        assert!(PolicyInput::parse(" ", None, true, None, PERMIT).is_err());
        assert!(PolicyInput::parse(&"a".repeat(101), None, true, None, PERMIT).is_err());
    }

    #[test]
    fn a_null_rule_is_no_rule() {
        let input = PolicyInput::parse("p", None, true, Some(Value::Null), PERMIT).unwrap();

        assert_eq!(input.rule, None);
    }

    #[test]
    fn a_realm_defaults_to_master_and_refuses_path_tricks() {
        assert_eq!(
            AgentSettingsInput::parse(true, None).unwrap().realm,
            "master"
        );
        for bad in ["", "..", "a/b", "a b"] {
            assert!(AgentSettingsInput::parse(true, Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_outcome_names_known_phases_only() {
        let ok = AgentsOutcome::parse(
            3,
            "Ready",
            true,
            None,
            None,
            vec![(Uuid::nil(), "Applied".to_string(), None)],
        );
        assert!(ok.is_ok());

        assert!(AgentsOutcome::parse(1, "Exploded", true, None, None, vec![]).is_err());
        assert!(
            AgentsOutcome::parse(
                1,
                "Ready",
                true,
                None,
                None,
                vec![(Uuid::nil(), "Pending".to_string(), None)]
            )
            .is_err()
        );
    }
}
