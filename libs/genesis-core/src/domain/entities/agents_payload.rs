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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentsPayloadV1 {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub enabled: bool,
    pub realm: String,
    pub revision: u64,
    #[serde(default)]
    pub policies: Vec<AgentPolicyDefinition>,
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

        Ok(())
    }
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
