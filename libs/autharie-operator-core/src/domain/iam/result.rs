use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplicationSummary {
    pub id: String,
    pub client_id: String,
    pub name: String,
    pub enabled: bool,
    pub client_type: String,
    pub public_client: bool,
    pub service_account_enabled: bool,
    pub protocol: String,
    pub registration_source: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplicationDetail {
    #[serde(flatten)]
    pub summary: ApplicationSummary,
    pub direct_access_grants_enabled: bool,
    pub require_pkce: bool,
    pub token_exchange_enabled: bool,
    pub consent_required: bool,
    pub redirect_uris: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token_lifetime: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token_lifetime: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token_lifetime: Option<i64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplicationPage {
    pub items: Vec<ApplicationSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum IamResult {
    Applications(ApplicationPage),
    Application(Box<ApplicationDetail>),
}

impl IamResult {
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn summary(id: &str) -> ApplicationSummary {
        ApplicationSummary {
            id: id.to_string(),
            client_id: format!("app-{id}"),
            name: format!("App {id}"),
            enabled: true,
            client_type: "confidential".to_string(),
            public_client: false,
            service_account_enabled: true,
            protocol: "openid-connect".to_string(),
            registration_source: "admin".to_string(),
            created_at: "2026-10-23T10:00:00Z".to_string(),
        }
    }

    pub fn detail(id: &str) -> ApplicationDetail {
        ApplicationDetail {
            summary: summary(id),
            direct_access_grants_enabled: false,
            require_pkce: true,
            token_exchange_enabled: false,
            consent_required: false,
            redirect_uris: vec!["https://app.example.com/callback".to_string()],
            access_token_lifetime: Some(300),
            refresh_token_lifetime: None,
            id_token_lifetime: None,
            updated_at: "2026-10-24T10:00:00Z".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::fixtures::*;
    use super::*;

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn a_summary_carries_exactly_the_frozen_fields() {
        let value = serde_json::to_value(summary("1")).unwrap();

        assert_eq!(
            keys(&value),
            [
                "client_id",
                "client_type",
                "created_at",
                "enabled",
                "id",
                "name",
                "protocol",
                "public_client",
                "registration_source",
                "service_account_enabled",
            ]
        );
    }

    #[test]
    fn a_detail_is_a_summary_and_the_frozen_extras_without_unset_lifetimes() {
        let value = serde_json::to_value(detail("1")).unwrap();

        assert_eq!(
            keys(&value),
            [
                "access_token_lifetime",
                "client_id",
                "client_type",
                "consent_required",
                "created_at",
                "direct_access_grants_enabled",
                "enabled",
                "id",
                "name",
                "protocol",
                "public_client",
                "redirect_uris",
                "registration_source",
                "require_pkce",
                "service_account_enabled",
                "token_exchange_enabled",
                "updated_at",
            ]
        );
        assert_eq!(
            value["redirect_uris"],
            json!(["https://app.example.com/callback"])
        );
    }

    #[test]
    fn a_page_names_the_next_page_only_when_there_is_one() {
        let last = IamResult::Applications(ApplicationPage {
            items: vec![summary("1")],
            next_page: None,
        });
        let more = IamResult::Applications(ApplicationPage {
            items: vec![summary("1")],
            next_page: Some(2),
        });

        let last = serde_json::from_slice::<Value>(&last.to_json().unwrap()).unwrap();
        let more = serde_json::from_slice::<Value>(&more.to_json().unwrap()).unwrap();

        assert!(last.get("next_page").is_none());
        assert_eq!(more["next_page"], json!(2));
        assert_eq!(more["items"][0]["id"], json!("1"));
    }

    #[test]
    fn neither_result_type_has_a_place_for_a_secret() {
        let page = IamResult::Applications(ApplicationPage {
            items: vec![summary("1")],
            next_page: None,
        });
        let one = IamResult::Application(Box::new(detail("1")));

        for result in [page, one] {
            let text = String::from_utf8(result.to_json().unwrap()).unwrap();
            assert!(!text.contains("secret"), "{text}");
        }
    }
}
