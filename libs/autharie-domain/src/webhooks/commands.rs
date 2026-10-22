use uuid::Uuid;

use crate::{
    dataplane::credential::SecretString,
    webhooks::{
        WebhookError,
        entities::{RequestPhase, WebhookState},
        value_objects::{
            HeaderInput, WebhookEndpoint, WebhookEvent, WebhookId, WebhookName, WebhookRealm,
            parse_description, parse_events, parse_headers,
        },
    },
};

#[derive(Debug, Clone)]
pub struct RawWebhookInput {
    pub realm: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub endpoint: String,
    pub events: Vec<String>,
    pub headers: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookInput {
    pub realm: WebhookRealm,
    pub name: WebhookName,
    pub description: Option<String>,
    pub endpoint: WebhookEndpoint,
    pub events: Vec<WebhookEvent>,
    pub headers: Vec<HeaderInput>,
}

impl WebhookInput {
    pub fn parse(raw: RawWebhookInput) -> Result<Self, WebhookError> {
        Ok(Self {
            realm: match raw.realm.as_deref() {
                Some(realm) => WebhookRealm::parse(realm)?,
                None => WebhookRealm::master(),
            },
            name: WebhookName::parse(&raw.name)?,
            description: parse_description(raw.description.as_deref())?,
            endpoint: WebhookEndpoint::parse(&raw.endpoint)?,
            events: parse_events(&raw.events)?,
            headers: parse_headers(&raw.headers)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookReport {
    pub id: WebhookId,
    pub state: WebhookState,
}

impl WebhookReport {
    pub fn parse(
        id: Uuid,
        phase: &str,
        ferriskey_id: Option<String>,
        message: Option<String>,
    ) -> Result<Self, WebhookError> {
        let state = match phase {
            "Pending" => WebhookState::Pending,
            "Active" => WebhookState::Active {
                ferriskey_id: ferriskey_id.filter(|id| !id.is_empty()).ok_or_else(|| {
                    WebhookError::invalid("an Active webhook needs a ferriskey_id")
                })?,
            },
            "Failed" => WebhookState::Failed {
                reason: message.unwrap_or_else(|| "the data plane gave no reason".to_string()),
            },
            other => {
                return Err(WebhookError::invalid(format!(
                    "'{other}' is not a webhook phase"
                )));
            }
        };

        Ok(Self {
            id: WebhookId(id),
            state,
        })
    }
}

pub struct SecretReport {
    pub id: WebhookId,
    pub version: u64,
    pub secret: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestReport {
    pub id: Uuid,
    pub phase: RequestPhase,
    pub message: Option<String>,
}

pub struct WebhooksOutcome {
    pub revision: u64,
    pub webhooks: Vec<WebhookReport>,
    pub secrets: Vec<SecretReport>,
    pub realms: Vec<String>,
    pub requests: Vec<RequestReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> RawWebhookInput {
        RawWebhookInput {
            realm: None,
            name: "orders".to_string(),
            description: None,
            endpoint: "https://example.com/hook".to_string(),
            events: vec!["user.created".to_string()],
            headers: vec![],
        }
    }

    #[test]
    fn a_missing_realm_is_master() {
        let input = WebhookInput::parse(raw()).unwrap();

        assert_eq!(input.realm, WebhookRealm::master());
    }

    #[test]
    fn an_http_endpoint_is_refused() {
        let mut http = raw();
        http.endpoint = "http://example.com/hook".to_string();

        assert!(WebhookInput::parse(http).is_err());
    }

    #[test]
    fn an_unknown_event_is_refused() {
        let mut unknown = raw();
        unknown.events = vec!["user.exploded".to_string()];

        assert!(WebhookInput::parse(unknown).is_err());
    }

    #[test]
    fn an_active_report_needs_its_ferriskey_id() {
        let id = Uuid::new_v4();

        assert!(WebhookReport::parse(id, "Active", None, None).is_err());
        assert_eq!(
            WebhookReport::parse(id, "Active", Some("fk".to_string()), None)
                .unwrap()
                .state,
            WebhookState::Active {
                ferriskey_id: "fk".to_string()
            }
        );
    }

    #[test]
    fn a_failed_report_keeps_its_reason() {
        let report =
            WebhookReport::parse(Uuid::new_v4(), "Failed", None, Some("ssrf".to_string())).unwrap();

        assert_eq!(
            report.state,
            WebhookState::Failed {
                reason: "ssrf".to_string()
            }
        );
    }

    #[test]
    fn an_unknown_phase_is_refused() {
        assert!(WebhookReport::parse(Uuid::new_v4(), "Exploded", None, None).is_err());
    }
}
