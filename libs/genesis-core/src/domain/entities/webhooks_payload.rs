use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::error::GenesisError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WebhookDefinition {
    pub id: Uuid,
    pub realm: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub endpoint: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub header_names: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct HeaderValues(BTreeMap<Uuid, BTreeMap<String, String>>);

impl fmt::Debug for HeaderValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HeaderValues")
            .field("webhooks", &self.0.len())
            .field("values", &"<redacted>")
            .finish()
    }
}

impl HeaderValues {
    pub fn secret_data(&self) -> BTreeMap<String, String> {
        self.0
            .iter()
            .flat_map(|(webhook, headers)| {
                headers
                    .iter()
                    .map(move |(name, value)| (format!("{webhook}.{name}"), value.clone()))
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhooksPayloadV1 {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub revision: u64,
    pub webhooks: Vec<WebhookDefinition>,
    pub headers: HeaderValues,
}

#[derive(Deserialize)]
struct RawWebhooksPayload {
    deployment_id: Uuid,
    namespace: String,
    revision: u64,
    webhooks: Vec<WebhookDefinition>,
    #[serde(default)]
    headers: Value,
}

impl WebhooksPayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let raw: RawWebhooksPayload =
            serde_json::from_value(payload.clone()).map_err(|error| invalid(error.to_string()))?;

        validate_definitions(&raw.webhooks)?;
        let headers = parse_headers(&raw.headers, &raw.webhooks)?;

        Ok(Self {
            deployment_id: raw.deployment_id,
            namespace: raw.namespace,
            revision: raw.revision,
            webhooks: raw.webhooks,
            headers,
        })
    }
}

fn validate_definitions(webhooks: &[WebhookDefinition]) -> Result<(), GenesisError> {
    let mut seen = BTreeSet::new();

    for webhook in webhooks {
        if !seen.insert(webhook.id) {
            return Err(invalid(format!("webhook {} appears twice", webhook.id)));
        }
        if webhook.realm.trim().is_empty() || webhook.name.trim().is_empty() {
            return Err(invalid(format!(
                "webhook {} needs a realm and a name",
                webhook.id
            )));
        }
        if !webhook.endpoint.starts_with("https://") || webhook.endpoint.len() <= "https://".len() {
            return Err(invalid(format!(
                "webhook {} endpoint is not an https URL",
                webhook.id
            )));
        }

        let mut names = BTreeSet::new();
        for name in &webhook.header_names {
            if !is_header_name(name) {
                return Err(invalid(format!(
                    "webhook {} has a header name that is not a plain token",
                    webhook.id
                )));
            }
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "webhook {} names header {name} twice",
                    webhook.id
                )));
            }
        }
    }

    Ok(())
}

fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn parse_headers(
    headers: &Value,
    webhooks: &[WebhookDefinition],
) -> Result<HeaderValues, GenesisError> {
    let entries = match headers {
        Value::Null => return check_all_present(BTreeMap::new(), webhooks),
        Value::Object(entries) => entries,
        _ => return Err(invalid("headers must be an object keyed by webhook id")),
    };

    let mut parsed: BTreeMap<Uuid, BTreeMap<String, String>> = BTreeMap::new();

    for (key, value) in entries {
        let id = Uuid::parse_str(key)
            .map_err(|_| invalid("headers has a key that is not a webhook id"))?;
        let definition = webhooks
            .iter()
            .find(|webhook| webhook.id == id)
            .ok_or_else(|| invalid(format!("headers for unknown webhook {id}")))?;
        let Value::Object(named) = value else {
            return Err(invalid(format!(
                "headers of webhook {id} must be an object of name to value"
            )));
        };

        let mut values = BTreeMap::new();
        for (name, value) in named {
            if !definition
                .header_names
                .iter()
                .any(|declared| declared == name)
            {
                return Err(invalid(format!(
                    "webhook {id} carries a value for header {name} it does not declare"
                )));
            }
            let Value::String(value) = value else {
                return Err(invalid(format!(
                    "the value of header {name} of webhook {id} must be a string"
                )));
            };
            values.insert(name.clone(), value.clone());
        }
        parsed.insert(id, values);
    }

    check_all_present(parsed, webhooks)
}

fn check_all_present(
    parsed: BTreeMap<Uuid, BTreeMap<String, String>>,
    webhooks: &[WebhookDefinition],
) -> Result<HeaderValues, GenesisError> {
    for webhook in webhooks {
        for name in &webhook.header_names {
            let present = parsed
                .get(&webhook.id)
                .is_some_and(|values| values.contains_key(name));
            if !present {
                return Err(invalid(format!(
                    "webhook {} declares header {name} without a value",
                    webhook.id
                )));
            }
        }
    }

    Ok(HeaderValues(parsed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookRequestKind {
    RotateSecret,
    RetryDelivery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRequestEntry {
    pub id: String,
    pub kind: WebhookRequestKind,
    pub webhook_id: String,
    pub delivery_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRequestPayload {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub entry: WebhookRequestEntry,
}

#[derive(Deserialize)]
struct RawRotate {
    deployment_id: Uuid,
    namespace: String,
    webhook_id: Uuid,
    request_id: Uuid,
}

#[derive(Deserialize)]
struct RawRetry {
    deployment_id: Uuid,
    namespace: String,
    webhook_id: Uuid,
    delivery_id: String,
    request_id: Uuid,
}

impl WebhookRequestPayload {
    pub fn rotate_from_value(payload: &Value) -> Result<Self, GenesisError> {
        let raw: RawRotate = decode(payload)?;

        Ok(Self {
            deployment_id: raw.deployment_id,
            namespace: raw.namespace,
            entry: WebhookRequestEntry {
                id: raw.request_id.to_string(),
                kind: WebhookRequestKind::RotateSecret,
                webhook_id: raw.webhook_id.to_string(),
                delivery_id: None,
            },
        })
    }

    pub fn retry_from_value(payload: &Value) -> Result<Self, GenesisError> {
        let raw: RawRetry = decode(payload)?;

        if raw.delivery_id.trim().is_empty() {
            return Err(invalid("delivery_id is empty"));
        }

        Ok(Self {
            deployment_id: raw.deployment_id,
            namespace: raw.namespace,
            entry: WebhookRequestEntry {
                id: raw.request_id.to_string(),
                kind: WebhookRequestKind::RetryDelivery,
                webhook_id: raw.webhook_id.to_string(),
                delivery_id: Some(raw.delivery_id),
            },
        })
    }
}

fn decode<T: for<'de> Deserialize<'de>>(payload: &Value) -> Result<T, GenesisError> {
    serde_json::from_value(payload.clone()).map_err(|error| invalid(error.to_string()))
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
    const HOOK: Uuid = Uuid::from_u128(2);
    const OTHER: Uuid = Uuid::from_u128(3);
    const VALUE: &str = "Bearer s3cr3t-value";

    fn webhook(id: Uuid, headers: &[&str]) -> Value {
        json!({
            "id": id,
            "realm": "master",
            "name": "audit",
            "description": null,
            "endpoint": "https://hooks.example.com/in",
            "events": ["user.created"],
            "header_names": headers,
        })
    }

    fn payload() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 4,
            "webhooks": [webhook(HOOK, &["Authorization"])],
            "headers": { HOOK.to_string(): { "Authorization": VALUE } },
        })
    }

    fn with(mutate: impl FnOnce(&mut Value)) -> Value {
        let mut value = payload();
        mutate(&mut value);
        value
    }

    #[test]
    fn a_valid_payload_is_decoded_whole() {
        let parsed = WebhooksPayloadV1::from_value(&payload()).expect("valid");

        assert_eq!(parsed.deployment_id, DEPLOYMENT);
        assert_eq!(parsed.namespace, "tenant-a");
        assert_eq!(parsed.revision, 4);
        assert_eq!(parsed.webhooks.len(), 1);
        assert_eq!(parsed.webhooks[0].header_names, vec!["Authorization"]);
    }

    #[test]
    fn header_values_become_secret_keys_named_after_webhook_and_header() {
        let parsed = WebhooksPayloadV1::from_value(&payload()).expect("valid");

        assert_eq!(
            parsed.headers.secret_data(),
            BTreeMap::from([(format!("{HOOK}.Authorization"), VALUE.to_string())])
        );
    }

    #[test]
    fn a_payload_without_headers_is_valid_when_no_webhook_declares_one() {
        let value = json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 1,
            "webhooks": [webhook(HOOK, &[])],
        });

        let parsed = WebhooksPayloadV1::from_value(&value).expect("valid");

        assert!(parsed.headers.secret_data().is_empty());
    }

    #[test]
    fn an_empty_set_is_valid_and_clears_everything() {
        let value = json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 9,
            "webhooks": [],
            "headers": {},
        });

        let parsed = WebhooksPayloadV1::from_value(&value).expect("valid");

        assert!(parsed.webhooks.is_empty());
        assert!(parsed.headers.secret_data().is_empty());
    }

    #[test]
    fn the_revision_is_required_and_must_be_a_non_negative_integer() {
        for bad in [json!(null), json!(-1), json!("4"), json!(1.5)] {
            let value = with(|v| v["revision"] = bad);
            assert!(WebhooksPayloadV1::from_value(&value).is_err());
        }
        let missing = with(|v| {
            v.as_object_mut().unwrap().remove("revision");
        });
        assert!(WebhooksPayloadV1::from_value(&missing).is_err());
    }

    #[test]
    fn a_webhook_id_must_be_a_uuid_and_unique() {
        let not_uuid = with(|v| v["webhooks"][0]["id"] = json!("abc"));
        assert!(WebhooksPayloadV1::from_value(&not_uuid).is_err());

        let twice = with(|v| {
            v["webhooks"] = json!([webhook(HOOK, &["Authorization"]), webhook(HOOK, &[])])
        });
        assert!(WebhooksPayloadV1::from_value(&twice).is_err());
    }

    #[test]
    fn only_https_endpoints_are_accepted() {
        for endpoint in ["http://hooks.example.com", "https://", "ftp://x", ""] {
            let value = with(|v| v["webhooks"][0]["endpoint"] = json!(endpoint));
            assert!(WebhooksPayloadV1::from_value(&value).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn the_header_map_must_have_the_frozen_shape() {
        for headers in [
            json!([]),
            json!("x"),
            json!({ "not-a-uuid": {} }),
            json!({ HOOK.to_string(): "x" }),
            json!({ HOOK.to_string(): { "Authorization": 5 } }),
            json!({ OTHER.to_string(): {} }),
            json!({ HOOK.to_string(): { "Authorization": VALUE, "X-Extra": "v" } }),
            json!({ HOOK.to_string(): {} }),
        ] {
            let value = with(|v| v["headers"] = headers.clone());
            assert!(WebhooksPayloadV1::from_value(&value).is_err(), "{headers}");
        }
    }

    #[test]
    fn header_names_are_plain_tokens_and_not_repeated() {
        for name in ["", "a.b", "a b", "a/b", "é"] {
            let value = with(|v| {
                v["webhooks"][0]["header_names"] = json!([name]);
                v["headers"] = json!({ HOOK.to_string(): { name: VALUE } });
            });
            assert!(WebhooksPayloadV1::from_value(&value).is_err(), "{name:?}");
        }

        let repeated = with(|v| {
            v["webhooks"][0]["header_names"] = json!(["X-A", "x-a"]);
            v["headers"] = json!({ HOOK.to_string(): { "X-A": "1", "x-a": "2" } });
        });
        assert!(WebhooksPayloadV1::from_value(&repeated).is_err());
    }

    #[test]
    fn a_header_value_never_appears_in_an_error_or_in_debug_output() {
        let malformed = [
            with(|v| v["headers"] = json!({ HOOK.to_string(): VALUE })),
            with(|v| v["headers"] = json!(VALUE)),
            with(|v| v["headers"] = json!({ VALUE: {} })),
            with(|v| v["headers"] = json!({ HOOK.to_string(): { "X": VALUE } })),
            with(|v| {
                v["headers"] = json!({ HOOK.to_string(): { "Authorization": VALUE, "X": VALUE } })
            }),
        ];
        for value in malformed {
            let error = WebhooksPayloadV1::from_value(&value).expect_err("invalid");
            assert!(!error.to_string().contains(VALUE), "{error}");
        }

        let parsed = WebhooksPayloadV1::from_value(&payload()).expect("valid");
        assert!(!format!("{parsed:?}").contains(VALUE));
    }

    fn rotate() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "webhook_id": HOOK,
            "request_id": OTHER,
        })
    }

    #[test]
    fn a_rotation_becomes_a_request_keyed_by_its_request_id() {
        let parsed = WebhookRequestPayload::rotate_from_value(&rotate()).expect("valid");

        assert_eq!(parsed.namespace, "tenant-a");
        assert_eq!(
            parsed.entry,
            WebhookRequestEntry {
                id: OTHER.to_string(),
                kind: WebhookRequestKind::RotateSecret,
                webhook_id: HOOK.to_string(),
                delivery_id: None,
            }
        );
    }

    #[test]
    fn a_retry_carries_the_delivery_id() {
        let mut value = rotate();
        value["delivery_id"] = json!("d-1");

        let parsed = WebhookRequestPayload::retry_from_value(&value).expect("valid");

        assert_eq!(parsed.entry.kind, WebhookRequestKind::RetryDelivery);
        assert_eq!(parsed.entry.delivery_id.as_deref(), Some("d-1"));
    }

    #[test]
    fn requests_need_uuid_ids_and_a_retry_needs_a_delivery() {
        let mut no_request = rotate();
        no_request["request_id"] = json!("nope");
        assert!(WebhookRequestPayload::rotate_from_value(&no_request).is_err());

        let mut bad_webhook = rotate();
        bad_webhook["webhook_id"] = json!(5);
        assert!(WebhookRequestPayload::rotate_from_value(&bad_webhook).is_err());

        assert!(WebhookRequestPayload::retry_from_value(&rotate()).is_err());

        let mut blank = rotate();
        blank["delivery_id"] = json!("  ");
        assert!(WebhookRequestPayload::retry_from_value(&blank).is_err());
    }
}
