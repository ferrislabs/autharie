use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::value_objects::{
    FromEmail, FromName, SmtpEncryption, SmtpHost, SmtpPort, SmtpRealm, SmtpUsername,
};
use crate::{dataplane::credential_repository::SealedSecret, deployments::DeploymentId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "phase")]
pub enum SmtpState {
    Pending,
    Applied,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SmtpSettings {
    pub host: SmtpHost,
    pub port: SmtpPort,
    pub encryption: SmtpEncryption,
    pub username: SmtpUsername,
    pub from_email: FromEmail,
    pub from_name: FromName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub deployment_id: DeploymentId,
    pub realm: SmtpRealm,
    pub settings: SmtpSettings,
    pub state: SmtpState,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSmtp {
    pub config: SmtpConfig,
    pub sealed_password: SealedSecret,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SmtpView {
    #[serde(flatten)]
    pub settings: SmtpSettings,
    pub password_set: bool,
    pub state: SmtpState,
    pub updated_at: DateTime<Utc>,
}

impl From<SmtpConfig> for SmtpView {
    fn from(config: SmtpConfig) -> Self {
        Self {
            settings: config.settings,
            password_set: true,
            state: config.state,
            updated_at: config.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_state_serialises_with_its_phase_and_its_message() {
        assert_eq!(
            serde_json::to_value(SmtpState::Pending).unwrap(),
            json!({ "phase": "Pending" })
        );
        assert_eq!(
            serde_json::to_value(SmtpState::Applied).unwrap(),
            json!({ "phase": "Applied" })
        );
        assert_eq!(
            serde_json::to_value(SmtpState::Failed {
                message: "refused".to_string()
            })
            .unwrap(),
            json!({ "phase": "Failed", "message": "refused" })
        );
    }

    #[test]
    fn a_view_is_flat_and_says_the_password_is_set() {
        let view = SmtpView {
            settings: SmtpSettings {
                host: SmtpHost::parse("smtp.example.com").unwrap(),
                port: SmtpPort::parse(587).unwrap(),
                encryption: SmtpEncryption::Starttls,
                username: SmtpUsername::parse("apikey").unwrap(),
                from_email: FromEmail::parse("no-reply@example.com").unwrap(),
                from_name: FromName::parse("Acme").unwrap(),
            },
            password_set: true,
            state: SmtpState::Pending,
            updated_at: Utc::now(),
        };

        let value = serde_json::to_value(view).unwrap();

        assert_eq!(value["host"], "smtp.example.com");
        assert_eq!(value["port"], 587);
        assert_eq!(value["encryption"], "starttls");
        assert_eq!(value["from_email"], "no-reply@example.com");
        assert_eq!(value["password_set"], true);
        assert_eq!(value["state"], json!({ "phase": "Pending" }));
        assert!(value.get("password").is_none());
    }
}
