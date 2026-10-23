use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{SmtpSettings, value_objects::SmtpEncryption};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpActionSettings {
    pub host: String,
    pub port: u16,
    pub encryption: SmtpEncryption,
    pub username: String,
    pub from_email: String,
    pub from_name: String,
    pub password: Option<String>,
}

impl SmtpActionSettings {
    pub fn of(settings: &SmtpSettings, password: Option<&str>) -> Self {
        Self {
            host: settings.host.as_str().to_string(),
            port: settings.port.value(),
            encryption: settings.encryption,
            username: settings.username.as_str().to_string(),
            from_email: settings.from_email.as_str().to_string(),
            from_name: settings.from_name.as_str().to_string(),
            password: password.map(str::to_string),
        }
    }
}

impl fmt::Debug for SmtpActionSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpActionSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("encryption", &self.encryption)
            .field("username", &self.username)
            .field("from_email", &self.from_email)
            .field("from_name", &self.from_name)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpActionPayload {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub revision: u64,
    pub realm: String,
    pub smtp: Option<SmtpActionSettings>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn payload(smtp: Option<SmtpActionSettings>) -> SmtpActionPayload {
        SmtpActionPayload {
            deployment_id: Uuid::nil(),
            namespace: "production-auth".to_string(),
            revision: 4,
            realm: "master".to_string(),
            smtp,
        }
    }

    fn settings(password: Option<&str>) -> SmtpActionSettings {
        SmtpActionSettings {
            host: "smtp.example.com".to_string(),
            port: 587,
            encryption: SmtpEncryption::Starttls,
            username: "apikey".to_string(),
            from_email: "no-reply@example.com".to_string(),
            from_name: "Acme".to_string(),
            password: password.map(str::to_string),
        }
    }

    #[test]
    fn a_save_carries_every_field_of_the_contract() {
        let value = serde_json::to_value(payload(Some(settings(Some("pw"))))).unwrap();

        assert_eq!(
            value,
            json!({
                "deployment_id": Uuid::nil(),
                "namespace": "production-auth",
                "revision": 4,
                "realm": "master",
                "smtp": {
                    "host": "smtp.example.com",
                    "port": 587,
                    "encryption": "starttls",
                    "username": "apikey",
                    "from_email": "no-reply@example.com",
                    "from_name": "Acme",
                    "password": "pw"
                }
            })
        );
    }

    #[test]
    fn keeping_the_password_is_a_null() {
        let value = serde_json::to_value(payload(Some(settings(None)))).unwrap();

        assert_eq!(value["smtp"]["password"], serde_json::Value::Null);
    }

    #[test]
    fn a_removal_is_a_null_smtp() {
        let value = serde_json::to_value(payload(None)).unwrap();

        assert!(value.get("smtp").is_some_and(serde_json::Value::is_null));
    }

    #[test]
    fn the_payload_does_not_print_its_password() {
        let printed = format!("{:?}", payload(Some(settings(Some("hunter2-canary")))));

        assert!(!printed.contains("hunter2"));
    }
}
