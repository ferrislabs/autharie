use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::tokens::ThemeTokens;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortalThemeActionPayload {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub revision: u64,
    pub realm: String,
    pub config: ThemeTokens,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn payload(config: serde_json::Value) -> PortalThemeActionPayload {
        PortalThemeActionPayload {
            deployment_id: Uuid::nil(),
            namespace: "production-auth".to_string(),
            revision: 4,
            realm: "master".to_string(),
            config: ThemeTokens::parse(&config).unwrap(),
        }
    }

    #[test]
    fn an_activation_carries_every_field_of_the_contract() {
        let value = serde_json::to_value(payload(json!({
            "colors": { "primaryButton": "#112233" },
            "borders": { "buttonRadius": 8, "widgetShadow": "none" }
        })))
        .unwrap();

        assert_eq!(
            value,
            json!({
                "deployment_id": Uuid::nil(),
                "namespace": "production-auth",
                "revision": 4,
                "realm": "master",
                "config": {
                    "colors": { "primaryButton": "#112233" },
                    "borders": { "buttonRadius": 8, "widgetShadow": "none" }
                }
            })
        );
    }

    #[test]
    fn no_active_theme_is_empty_tokens_not_a_null() {
        let value = serde_json::to_value(payload(json!({}))).unwrap();

        assert_eq!(value["config"], json!({}));
    }
}
