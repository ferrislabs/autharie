use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
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

const MAX_SERVERS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentServerDefinition {
    pub id: Uuid,
    pub slug: String,
    pub url: String,
    pub enabled: bool,
    #[serde(default)]
    pub discovery_requested_at: Option<DateTime<Utc>>,
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
    pub servers: Vec<AgentServerDefinition>,
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

        self.validate_servers()
    }

    fn validate_servers(&self) -> Result<(), GenesisError> {
        if self.servers.len() > MAX_SERVERS {
            return Err(invalid(format!("more than {MAX_SERVERS} servers")));
        }

        let mut ids = BTreeSet::new();
        let mut slugs = BTreeSet::new();
        for server in &self.servers {
            if !is_server_slug(&server.slug) {
                return Err(invalid(format!("server {} has an invalid slug", server.id)));
            }
            if !server.url.starts_with("https://") {
                return Err(invalid(format!("server {} needs an https URL", server.id)));
            }
            if !ids.insert(server.id) || !slugs.insert(server.slug.as_str()) {
                return Err(invalid(format!("server {} appears twice", server.id)));
            }
        }

        Ok(())
    }
}

fn is_server_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    (2..=31).contains(&slug.len())
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !matches!(slug, "ferriskey" | "autharie" | "gateway")
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

    const SERVER: Uuid = Uuid::from_u128(7);

    fn server(slug: &str) -> Value {
        json!({
            "id": SERVER,
            "slug": slug,
            "url": "https://tools.example.com/mcp",
            "enabled": true,
            "discovery_requested_at": "2026-10-23T10:00:00Z",
        })
    }

    #[test]
    fn a_payload_without_servers_is_valid_and_has_none() {
        let parsed = AgentsPayloadV1::from_value(&payload()).expect("valid");

        assert!(parsed.servers.is_empty());
    }

    #[test]
    fn customer_servers_are_decoded_with_their_discovery_request() {
        let value = with(|value| {
            value["servers"] = json!([server("crm"), {
                "id": Uuid::from_u128(8),
                "slug": "erp",
                "url": "https://erp.example.com/mcp",
                "enabled": false,
            }]);
        });

        let parsed = AgentsPayloadV1::from_value(&value).expect("valid");

        assert_eq!(parsed.servers.len(), 2);
        assert_eq!(parsed.servers[0].id, SERVER);
        assert_eq!(parsed.servers[0].slug, "crm");
        assert!(parsed.servers[0].enabled);
        assert!(parsed.servers[0].discovery_requested_at.is_some());
        assert_eq!(parsed.servers[1].discovery_requested_at, None);
        assert!(!parsed.servers[1].enabled);
    }

    #[test]
    fn a_server_with_a_bad_slug_a_plain_url_or_a_twin_is_refused() {
        for slug in ["", "a", "Crm", "1crm", "cr m", "ferriskey", "gateway"] {
            let value = with(|value| value["servers"] = json!([server(slug)]));
            assert!(AgentsPayloadV1::from_value(&value).is_err(), "{slug:?}");
        }
        let plain = with(|value| {
            let mut bad = server("crm");
            bad["url"] = json!("http://tools.example.com/mcp");
            value["servers"] = json!([bad]);
        });
        let twin = with(|value| value["servers"] = json!([server("crm"), server("crm")]));

        assert!(AgentsPayloadV1::from_value(&plain).is_err());
        assert!(AgentsPayloadV1::from_value(&twin).is_err());
    }

    #[test]
    fn more_than_ten_servers_are_refused() {
        let many: Vec<Value> = (0..11)
            .map(|index| {
                json!({
                    "id": Uuid::from_u128(100 + index),
                    "slug": format!("srv-{index}"),
                    "url": "https://tools.example.com/mcp",
                    "enabled": true,
                })
            })
            .collect();
        let value = with(|value| value["servers"] = json!(many));

        assert!(AgentsPayloadV1::from_value(&value).is_err());
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
