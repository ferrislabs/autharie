use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::entities::smtp_payload::is_secret_key;
use crate::domain::error::GenesisError;

const DEFAULT_PAGE: u32 = 1;
const DEFAULT_LIMIT: u32 = 25;
const KEY_TYPE: &str = "RSA";
const KEY_ALGORITHM: &str = "RSA-OAEP-256";

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum Operation {
    #[serde(rename = "applications.list")]
    List {
        #[serde(default = "first_page")]
        page: u32,
        #[serde(default = "default_limit")]
        limit: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        search: Option<String>,
    },
    #[serde(rename = "applications.get")]
    Get { id: Uuid },
}

fn first_page() -> u32 {
    DEFAULT_PAGE
}

fn default_limit() -> u32 {
    DEFAULT_LIMIT
}

#[derive(Serialize, Deserialize)]
struct SealingKey {
    kty: String,
    n: String,
    e: String,
    alg: String,
}

#[derive(Deserialize)]
struct RawIamRequest {
    deployment_id: Uuid,
    namespace: String,
    request_id: Uuid,
    realm: String,
    operation: Operation,
    sealing_key: SealingKey,
    expires_at: DateTime<Utc>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct IamRequestEntry {
    pub request_id: String,
    pub realm: String,
    pub operation: String,
    pub sealing_key: String,
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for IamRequestEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IamRequestEntry")
            .field("request_id", &self.request_id)
            .field("realm", &self.realm)
            .field("operation", &"<redacted>")
            .field("sealing_key", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IamRequestPayloadV1 {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub entry: IamRequestEntry,
}

impl IamRequestPayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let raw = RawIamRequest::deserialize(payload).map_err(invalid)?;

        if !is_secret_key(&raw.realm) {
            return Err(invalid("realm must be a non-empty plain name"));
        }
        if raw.sealing_key.kty != KEY_TYPE || raw.sealing_key.alg != KEY_ALGORITHM {
            return Err(invalid("the sealing key must be an RSA-OAEP-256 key"));
        }
        if raw.sealing_key.n.is_empty() || raw.sealing_key.e.is_empty() {
            return Err(invalid("the sealing key has no modulus or exponent"));
        }

        Ok(Self {
            deployment_id: raw.deployment_id,
            namespace: raw.namespace,
            entry: IamRequestEntry {
                request_id: raw.request_id.hyphenated().to_string(),
                realm: raw.realm,
                operation: serde_json::to_string(&raw.operation).map_err(invalid)?,
                sealing_key: serde_json::to_string(&raw.sealing_key).map_err(invalid)?,
                expires_at: raw.expires_at,
            },
        })
    }
}

fn invalid(message: impl ToString) -> GenesisError {
    GenesisError::InvalidPayload {
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const REQUEST: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";
    const APP: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a22";

    fn payload(operation: Value) -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "request_id": REQUEST,
            "realm": "master",
            "operation": operation,
            "sealing_key": { "kty": "RSA", "n": "AQAB", "e": "AQAB", "alg": "RSA-OAEP-256" },
            "expires_at": "2026-10-24T12:05:00Z",
        })
    }

    fn list() -> Value {
        payload(json!({ "type": "applications.list", "page": 2, "limit": 50, "search": "web" }))
    }

    fn parse(value: &Value) -> Result<IamRequestPayloadV1, GenesisError> {
        IamRequestPayloadV1::from_value(value)
    }

    #[test]
    fn a_list_request_becomes_an_entry_with_the_operation_as_a_json_string() {
        let parsed = parse(&list()).expect("valid");

        assert_eq!(parsed.deployment_id, DEPLOYMENT);
        assert_eq!(parsed.namespace, "tenant-a");
        assert_eq!(parsed.entry.request_id, REQUEST);
        assert_eq!(parsed.entry.realm, "master");
        let operation: Value = serde_json::from_str(&parsed.entry.operation).unwrap();
        assert_eq!(
            operation,
            json!({ "type": "applications.list", "page": 2, "limit": 50, "search": "web" })
        );
        assert_eq!(
            parsed.entry.expires_at.to_rfc3339(),
            "2026-10-24T12:05:00+00:00"
        );
    }

    #[test]
    fn a_list_without_page_limit_or_search_takes_the_defaults() {
        let parsed = parse(&payload(json!({ "type": "applications.list" }))).expect("valid");

        let operation: Value = serde_json::from_str(&parsed.entry.operation).unwrap();
        assert_eq!(
            operation,
            json!({ "type": "applications.list", "page": 1, "limit": 25 })
        );
    }

    #[test]
    fn a_get_carries_the_application_id() {
        let parsed =
            parse(&payload(json!({ "type": "applications.get", "id": APP }))).expect("valid");

        let operation: Value = serde_json::from_str(&parsed.entry.operation).unwrap();
        assert_eq!(operation, json!({ "type": "applications.get", "id": APP }));
    }

    #[test]
    fn the_sealing_key_keeps_only_the_four_public_fields() {
        let mut value = list();
        value["sealing_key"]["ext"] = json!(true);
        value["sealing_key"]["key_ops"] = json!(["encrypt"]);
        value["sealing_key"]["d"] = json!("must-not-be-here");

        let parsed = parse(&value).expect("valid");

        let key: Value = serde_json::from_str(&parsed.entry.sealing_key).unwrap();
        assert_eq!(
            key,
            json!({ "kty": "RSA", "n": "AQAB", "e": "AQAB", "alg": "RSA-OAEP-256" })
        );
        assert!(!parsed.entry.sealing_key.contains("must-not-be-here"));
    }

    #[test]
    fn a_request_id_in_another_spelling_is_stored_lowercase_and_hyphenated() {
        let mut value = list();
        value["request_id"] = json!("0B9AD3C44A5F4C3B8D570B2D6D9E1A11");

        assert_eq!(parse(&value).expect("valid").entry.request_id, REQUEST);
    }

    #[test]
    fn what_is_not_a_named_operation_is_refused() {
        for operation in [
            json!({ "type": "applications.delete", "id": APP }),
            json!({ "type": "users.list" }),
            json!({ "type": "applications.get" }),
            json!({ "type": "applications.get", "id": "../../admin" }),
            json!({ "path": "/realms/master/clients" }),
            json!("applications.list"),
        ] {
            assert!(
                matches!(
                    parse(&payload(operation.clone())),
                    Err(GenesisError::InvalidPayload { .. })
                ),
                "{operation}"
            );
        }
    }

    #[test]
    fn a_key_that_is_not_an_rsa_oaep_256_key_is_refused() {
        for (field, value) in [("kty", "EC"), ("alg", "RSA-OAEP"), ("n", ""), ("e", "")] {
            let mut bad = list();
            bad["sealing_key"][field] = json!(value);
            assert!(parse(&bad).is_err(), "{field}={value}");
        }
        let mut missing = list();
        missing["sealing_key"] = json!({ "kty": "RSA" });
        assert!(parse(&missing).is_err());
    }

    #[test]
    fn a_payload_with_an_unusable_identifier_or_date_is_refused() {
        for (field, value) in [
            ("request_id", json!("not-a-uuid")),
            ("deployment_id", json!("nope")),
            ("realm", json!("a/b")),
            ("realm", json!("")),
            ("expires_at", json!("soon")),
            ("expires_at", json!(null)),
            ("namespace", json!(7)),
        ] {
            let mut bad = list();
            bad[field] = value.clone();
            assert!(parse(&bad).is_err(), "{field}={value}");
        }
    }

    #[test]
    fn the_search_term_and_the_key_never_reach_a_log_line() {
        let parsed = parse(&list()).expect("valid");

        let shown = format!("{parsed:?}");

        assert!(!shown.contains("web"));
        assert!(!shown.contains("AQAB"));
        assert!(shown.contains(REQUEST));
    }
}
