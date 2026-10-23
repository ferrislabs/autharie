use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::error::GenesisError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentPolicyDefinition {
    pub id: Uuid,
    pub name: String,
    pub cedar: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AgentAccessPayload {
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub approved: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SealingKeyPayload {
    pub kty: String,
    pub n: String,
    pub e: String,
    pub alg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AutonomousAgentPayload {
    pub client_id: String,
    pub name: String,
    #[serde(default)]
    pub request_id: Option<Uuid>,
    #[serde(default)]
    pub rotate_request_id: Option<Uuid>,
    #[serde(default)]
    pub sealing_key: Option<SealingKeyPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentsPayloadV1 {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub enabled: bool,
    pub realm: String,
    pub revision: u64,
    #[serde(default)]
    pub policies: Vec<AgentPolicyDefinition>,
    #[serde(default)]
    pub access: AgentAccessPayload,
    #[serde(default)]
    pub autonomous: Vec<AutonomousAgentPayload>,
    #[serde(default)]
    pub deleted_agents: Vec<String>,
}

impl AgentsPayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let parsed: Self =
            serde_json::from_value(payload.clone()).map_err(|error| invalid(error.to_string()))?;

        parsed.validate()?;
        Ok(parsed)
    }

    fn validate(&self) -> Result<(), GenesisError> {
        if !is_realm_name(&self.realm) {
            return Err(invalid("realm is not a valid realm name"));
        }

        let mut seen = BTreeSet::new();
        for policy in &self.policies {
            if !seen.insert(policy.id) {
                return Err(invalid(format!("policy {} appears twice", policy.id)));
            }
            if policy.name.trim().is_empty() {
                return Err(invalid(format!("policy {} needs a name", policy.id)));
            }
            if policy.cedar.trim().is_empty() {
                return Err(invalid(format!("policy {} has no Cedar source", policy.id)));
            }
        }

        for id in self
            .access
            .approved
            .iter()
            .chain(&self.access.blocked)
            .chain(&self.deleted_agents)
        {
            if !is_client_id(id) {
                return Err(invalid("an agent access list names an invalid client id"));
            }
        }

        let mut clients = BTreeSet::new();
        for agent in &self.autonomous {
            if !is_client_id(&agent.client_id) || !clients.insert(agent.client_id.as_str()) {
                return Err(invalid(
                    "an autonomous agent has an invalid or repeated client id",
                ));
            }
            if agent.name.trim().is_empty() {
                return Err(invalid("an autonomous agent needs a name"));
            }
            if agent.request_id.is_some() && agent.rotate_request_id.is_some() {
                return Err(invalid(
                    "an autonomous agent cannot be created and rotated at once",
                ));
            }
            let asked = agent.request_id.or(agent.rotate_request_id).is_some();
            if asked != agent.sealing_key.is_some() {
                return Err(invalid(
                    "an autonomous agent request needs its sealing key, and only then",
                ));
            }
            if agent
                .sealing_key
                .as_ref()
                .is_some_and(|key| key.kty != "RSA" || key.alg != "RSA-OAEP-256")
            {
                return Err(invalid("a sealing key must be an RSA-OAEP-256 key"));
            }
        }

        Ok(())
    }
}

fn is_client_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= 512
        && id
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && c != '"' && c != '\\')
}

fn is_realm_name(realm: &str) -> bool {
    !realm.is_empty()
        && realm != "."
        && realm != ".."
        && realm
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn invalid(message: impl Into<String>) -> GenesisError {
    GenesisError::InvalidPayload {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const POLICY: Uuid = Uuid::from_u128(2);
    const OTHER: Uuid = Uuid::from_u128(3);

    fn payload() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "enabled": true,
            "realm": "master",
            "revision": 4,
            "policies": [{
                "id": POLICY,
                "name": "Read only",
                "cedar": "permit (principal, action, resource);",
            }],
        })
    }

    fn with(mutate: impl FnOnce(&mut Value)) -> Value {
        let mut value = payload();
        mutate(&mut value);
        value
    }

    #[test]
    fn a_valid_payload_is_decoded_whole() {
        let parsed = AgentsPayloadV1::from_value(&payload()).expect("valid");

        assert_eq!(parsed.deployment_id, DEPLOYMENT);
        assert_eq!(parsed.namespace, "tenant-a");
        assert!(parsed.enabled);
        assert_eq!(parsed.realm, "master");
        assert_eq!(parsed.revision, 4);
        assert_eq!(parsed.policies.len(), 1);
        assert_eq!(parsed.policies[0].id, POLICY);
        assert_eq!(parsed.policies[0].name, "Read only");
    }

    #[test]
    fn a_disabled_payload_without_policies_is_valid() {
        let parsed = AgentsPayloadV1::from_value(&with(|value| {
            value["enabled"] = json!(false);
            value.as_object_mut().expect("object").remove("policies");
        }))
        .expect("valid");

        assert!(!parsed.enabled);
        assert!(parsed.policies.is_empty());
    }

    #[test]
    fn the_contract_fields_are_required() {
        for key in ["deployment_id", "namespace", "enabled", "realm", "revision"] {
            let value = with(|value| {
                value.as_object_mut().expect("object").remove(key);
            });

            assert!(AgentsPayloadV1::from_value(&value).is_err(), "{key}");
        }
    }

    #[test]
    fn a_realm_that_is_not_a_slug_is_refused() {
        for realm in ["", "..", "a/b", "a b", "a?x=1"] {
            let value = with(|value| value["realm"] = json!(realm));

            assert!(AgentsPayloadV1::from_value(&value).is_err(), "{realm:?}");
        }
    }

    #[test]
    fn a_policy_listed_twice_is_refused() {
        let value = with(|value| {
            let first = value["policies"][0].clone();
            value["policies"].as_array_mut().expect("array").push(first);
        });

        assert!(AgentsPayloadV1::from_value(&value).is_err());
    }

    #[test]
    fn a_policy_needs_an_id_a_name_and_a_source() {
        let blank_name = with(|value| value["policies"][0]["name"] = json!(" "));
        let blank_cedar = with(|value| value["policies"][0]["cedar"] = json!(""));
        let bad_id = with(|value| value["policies"][0]["id"] = json!("not-a-uuid"));

        assert!(AgentsPayloadV1::from_value(&blank_name).is_err());
        assert!(AgentsPayloadV1::from_value(&blank_cedar).is_err());
        assert!(AgentsPayloadV1::from_value(&bad_id).is_err());
    }

    #[test]
    fn two_distinct_policies_are_accepted() {
        let value = with(|value| {
            value["policies"]
                .as_array_mut()
                .expect("array")
                .push(json!({
                    "id": OTHER,
                    "name": "Deny deletes",
                    "cedar": "forbid (principal, action, resource);",
                }));
        });

        assert_eq!(
            AgentsPayloadV1::from_value(&value)
                .expect("valid")
                .policies
                .len(),
            2
        );
    }

    fn registry(mutate: impl FnOnce(&mut Value)) -> Value {
        with(|value| {
            value["access"] = json!({
                "approval_required": true, "approved": ["a"], "blocked": ["b"]
            });
            value["autonomous"] = json!([{
                "client_id": "agent-x-abc123", "name": "X", "request_id": OTHER,
                "sealing_key": { "kty": "RSA", "n": "AA", "e": "AQAB", "alg": "RSA-OAEP-256" }
            }]);
            value["deleted_agents"] = json!(["old"]);
            mutate(value);
        })
    }

    #[test]
    fn the_registry_fields_are_decoded() {
        let parsed = AgentsPayloadV1::from_value(&registry(|_| {})).expect("valid");

        assert!(parsed.access.approval_required);
        assert_eq!(parsed.access.approved, vec!["a".to_string()]);
        assert_eq!(parsed.access.blocked, vec!["b".to_string()]);
        assert_eq!(parsed.autonomous[0].request_id, Some(OTHER));
        assert_eq!(parsed.deleted_agents, vec!["old".to_string()]);
    }

    #[test]
    fn a_payload_without_the_registry_fields_is_open_and_empty() {
        let parsed = AgentsPayloadV1::from_value(&payload()).expect("valid");

        assert_eq!(parsed.access, AgentAccessPayload::default());
        assert!(parsed.autonomous.is_empty());
        assert!(parsed.deleted_agents.is_empty());
    }

    #[test]
    fn an_invalid_registry_payload_is_refused() {
        type Edit = Box<dyn Fn(&mut Value)>;
        let cases: Vec<Edit> = vec![
            Box::new(|v| v["access"]["approved"] = json!(["a b"])),
            Box::new(|v| v["deleted_agents"] = json!([""])),
            Box::new(|v| v["autonomous"][0]["client_id"] = json!("")),
            Box::new(|v| v["autonomous"][0]["name"] = json!(" ")),
            Box::new(|v| {
                let first = v["autonomous"][0].clone();
                v["autonomous"].as_array_mut().unwrap().push(first);
            }),
            Box::new(|v| v["autonomous"][0]["rotate_request_id"] = json!(POLICY)),
            Box::new(|v| {
                v["autonomous"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("sealing_key");
            }),
            Box::new(|v| {
                v["autonomous"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("request_id");
            }),
            Box::new(|v| v["autonomous"][0]["sealing_key"]["kty"] = json!("EC")),
            Box::new(|v| v["autonomous"][0]["sealing_key"]["alg"] = json!("RSA-OAEP")),
        ];

        for case in cases {
            assert!(AgentsPayloadV1::from_value(&registry(case)).is_err());
        }
    }

    #[test]
    fn a_settled_autonomous_agent_has_no_request_and_no_key() {
        let value = registry(|v| {
            v["autonomous"] = json!([{ "client_id": "agent-x-abc123", "name": "X" }]);
        });

        assert!(AgentsPayloadV1::from_value(&value).is_ok());
    }

    #[test]
    fn a_refusal_never_quotes_the_cedar_source() {
        let value = with(|value| {
            value["policies"][0]["name"] = json!("");
            value["policies"][0]["cedar"] = json!("permit (principal == User::\"secret-user\");");
        });

        let error = AgentsPayloadV1::from_value(&value).expect_err("invalid");

        assert!(!error.to_string().contains("secret-user"));
    }
}
