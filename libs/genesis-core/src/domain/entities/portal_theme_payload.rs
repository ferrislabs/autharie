use serde::Deserialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::domain::entities::smtp_payload::is_secret_key;
use crate::domain::error::GenesisError;

pub const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalThemePayloadV1 {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub revision: u64,
    pub realm: String,
    pub config: Map<String, Value>,
}

#[derive(Deserialize)]
struct RawPortalThemePayload {
    deployment_id: Uuid,
    namespace: String,
    revision: u64,
    realm: String,
    config: Value,
}

impl PortalThemePayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let raw = RawPortalThemePayload::deserialize(payload).map_err(invalid)?;

        if !is_secret_key(&raw.realm) {
            return Err(invalid("realm must be a non-empty plain name"));
        }

        let Value::Object(config) = raw.config else {
            return Err(invalid("config must be a JSON object"));
        };

        let size = serde_json::to_vec(&config).map_err(invalid)?.len();
        if size > MAX_CONFIG_BYTES {
            return Err(invalid(format!(
                "config is {size} bytes, at most {MAX_CONFIG_BYTES} are accepted"
            )));
        }

        Ok(Self {
            deployment_id: raw.deployment_id,
            namespace: raw.namespace,
            revision: raw.revision,
            realm: raw.realm,
            config,
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

    fn theme() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 4,
            "realm": "master",
            "config": { "colors": { "primary": "#112233" }, "borders": { "radius": 8 } },
        })
    }

    #[test]
    fn a_theme_is_parsed_whole() {
        let payload = PortalThemePayloadV1::from_value(&theme()).expect("valid");

        assert_eq!(payload.deployment_id, DEPLOYMENT);
        assert_eq!(payload.namespace, "tenant-a");
        assert_eq!(payload.revision, 4);
        assert_eq!(payload.realm, "master");
        assert_eq!(
            Value::Object(payload.config),
            json!({ "colors": { "primary": "#112233" }, "borders": { "radius": 8 } })
        );
    }

    #[test]
    fn an_empty_config_is_the_default_look_and_is_valid() {
        let mut value = theme();
        value["config"] = json!({});

        let payload = PortalThemePayloadV1::from_value(&value).expect("valid");

        assert!(payload.config.is_empty());
    }

    #[test]
    fn a_config_that_is_not_an_object_is_refused() {
        for config in [json!(null), json!([]), json!("{}"), json!(3), json!(true)] {
            let mut value = theme();
            value["config"] = config;

            assert!(matches!(
                PortalThemePayloadV1::from_value(&value),
                Err(GenesisError::InvalidPayload { .. })
            ));
        }
    }

    #[test]
    fn a_missing_config_is_refused() {
        let mut value = theme();
        value.as_object_mut().expect("object").remove("config");

        assert!(matches!(
            PortalThemePayloadV1::from_value(&value),
            Err(GenesisError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn a_config_over_64_kib_is_refused() {
        let mut value = theme();
        value["config"] = json!({ "filler": "x".repeat(MAX_CONFIG_BYTES) });

        assert!(matches!(
            PortalThemePayloadV1::from_value(&value),
            Err(GenesisError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn a_config_of_exactly_64_kib_is_accepted() {
        let overhead = serde_json::to_vec(&json!({ "filler": "" }))
            .expect("serialises")
            .len();
        let mut value = theme();
        value["config"] = json!({ "filler": "x".repeat(MAX_CONFIG_BYTES - overhead) });

        assert!(PortalThemePayloadV1::from_value(&value).is_ok());
    }

    #[test]
    fn an_empty_or_odd_realm_is_refused() {
        for realm in ["", "  ", "a/b", "two words"] {
            let mut value = theme();
            value["realm"] = json!(realm);

            assert!(matches!(
                PortalThemePayloadV1::from_value(&value),
                Err(GenesisError::InvalidPayload { .. })
            ));
        }
    }

    #[test]
    fn a_revision_that_is_not_a_number_is_refused() {
        for revision in [json!(-1), json!("4"), json!(null)] {
            let mut value = theme();
            value["revision"] = revision;

            assert!(matches!(
                PortalThemePayloadV1::from_value(&value),
                Err(GenesisError::InvalidPayload { .. })
            ));
        }
    }
}
