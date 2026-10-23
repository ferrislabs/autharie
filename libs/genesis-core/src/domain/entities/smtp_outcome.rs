use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    #[default]
    Smtp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpOutcomeItem {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpOutcomeReport {
    pub deployment_id: Uuid,
    #[serde(default)]
    kind: Kind,
    pub revision: u64,
    pub realm: String,
    pub items: Vec<SmtpOutcomeItem>,
}

impl SmtpOutcomeReport {
    pub fn new(
        deployment_id: Uuid,
        revision: u64,
        realm: impl Into<String>,
        item: SmtpOutcomeItem,
    ) -> Self {
        Self {
            deployment_id,
            kind: Kind::Smtp,
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
        let report = SmtpOutcomeReport::new(
            DEPLOYMENT,
            4,
            "master",
            SmtpOutcomeItem {
                phase: "Applied".to_string(),
                message: None,
            },
        );

        assert_eq!(
            serde_json::to_value(&report).expect("serialises"),
            json!({
                "deployment_id": DEPLOYMENT,
                "kind": "smtp",
                "revision": 4,
                "realm": "master",
                "items": [{ "phase": "Applied" }],
            })
        );
    }

    #[test]
    fn a_failure_carries_its_message() {
        let report = SmtpOutcomeReport::new(
            DEPLOYMENT,
            2,
            "acme",
            SmtpOutcomeItem {
                phase: "Failed".to_string(),
                message: Some("no password".to_string()),
            },
        );

        let value = serde_json::to_value(&report).expect("serialises");

        assert_eq!(
            value["items"],
            json!([{ "phase": "Failed", "message": "no password" }])
        );
    }
}
