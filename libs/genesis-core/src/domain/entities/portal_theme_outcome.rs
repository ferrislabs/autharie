use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    #[default]
    PortalTheme,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalThemeOutcomeItem {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalThemeOutcomeReport {
    pub deployment_id: Uuid,
    #[serde(default)]
    kind: Kind,
    pub revision: u64,
    pub realm: String,
    pub items: Vec<PortalThemeOutcomeItem>,
}

impl PortalThemeOutcomeReport {
    pub fn new(
        deployment_id: Uuid,
        revision: u64,
        realm: impl Into<String>,
        item: PortalThemeOutcomeItem,
    ) -> Self {
        Self {
            deployment_id,
            kind: Kind::PortalTheme,
            revision,
            realm: realm.into(),
            items: vec![item],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    #[test]
    fn it_serialises_in_the_frozen_shape() {
        let report = PortalThemeOutcomeReport::new(
            DEPLOYMENT,
            4,
            "master",
            PortalThemeOutcomeItem {
                phase: "Applied".to_string(),
                message: None,
            },
        );

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "portal_theme",
                "revision": 4,
                "realm": "master",
                "items": [{ "phase": "Applied" }],
            })
        );
    }

    #[test]
    fn a_failure_carries_its_message() {
        let report = PortalThemeOutcomeReport::new(
            DEPLOYMENT,
            2,
            "acme",
            PortalThemeOutcomeItem {
                phase: "Failed".to_string(),
                message: Some("404 Not Found".to_string()),
            },
        );

        let value = serde_json::to_value(&report).expect("serialises");

        assert_eq!(
            value["items"],
            json!([{ "phase": "Failed", "message": "404 Not Found" }])
        );
    }
}
