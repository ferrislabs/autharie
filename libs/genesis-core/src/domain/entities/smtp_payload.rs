use std::fmt;

use serde::{Deserialize, Deserializer};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::error::GenesisError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpEncryption {
    Tls,
    Starttls,
    None,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Password(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for Password {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(password) => Ok(Self(password)),
            _ => Err(serde::de::Error::custom("password must be a string")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub encryption: SmtpEncryption,
    pub username: String,
    pub from_email: String,
    pub from_name: String,
    #[serde(default)]
    pub password: Option<Password>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordChange<'a> {
    Set(&'a str),
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpPayloadV1 {
    pub deployment_id: Uuid,
    pub namespace: String,
    pub revision: u64,
    pub realm: String,
    pub smtp: Option<SmtpSettings>,
}

#[derive(Deserialize)]
struct RawSmtpPayload {
    deployment_id: Uuid,
    namespace: String,
    revision: u64,
    realm: String,
    smtp: Value,
}

impl SmtpPayloadV1 {
    pub fn from_value(payload: &Value) -> Result<Self, GenesisError> {
        let raw = RawSmtpPayload::deserialize(payload).map_err(invalid)?;

        if !is_secret_key(&raw.realm) {
            return Err(invalid("realm must be a non-empty plain name"));
        }

        let smtp = match &raw.smtp {
            Value::Null => None,
            settings => Some(SmtpSettings::deserialize(settings).map_err(invalid)?),
        };

        if let Some(settings) = &smtp {
            if settings.port == 0 {
                return Err(invalid("port must be between 1 and 65535"));
            }
            if settings
                .password
                .as_ref()
                .is_some_and(|password| password.expose().is_empty())
            {
                return Err(invalid("password must not be empty"));
            }
        }

        Ok(Self {
            deployment_id: raw.deployment_id,
            namespace: raw.namespace,
            revision: raw.revision,
            realm: raw.realm,
            smtp,
        })
    }

    pub fn password_change(&self) -> Option<PasswordChange<'_>> {
        match &self.smtp {
            None => Some(PasswordChange::Remove),
            Some(settings) => settings
                .password
                .as_ref()
                .map(|password| PasswordChange::Set(password.expose())),
        }
    }
}

pub(crate) fn is_secret_key(realm: &str) -> bool {
    !realm.is_empty()
        && realm.len() <= 253
        && realm
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
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
    const PASSWORD: &str = "hunter2-do-not-print";

    fn save() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 4,
            "realm": "master",
            "smtp": {
                "host": "smtp.example.com",
                "port": 587,
                "encryption": "starttls",
                "username": "mailer",
                "from_email": "no-reply@example.com",
                "from_name": "Example",
                "password": PASSWORD,
            },
        })
    }

    fn removal() -> Value {
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 5,
            "realm": "master",
            "smtp": null,
        })
    }

    #[test]
    fn a_save_is_parsed_whole() {
        let payload = SmtpPayloadV1::from_value(&save()).expect("valid");

        assert_eq!(payload.deployment_id, DEPLOYMENT);
        assert_eq!(payload.namespace, "tenant-a");
        assert_eq!(payload.revision, 4);
        assert_eq!(payload.realm, "master");
        let settings = payload.smtp.as_ref().expect("a save");
        assert_eq!(settings.host, "smtp.example.com");
        assert_eq!(settings.port, 587);
        assert_eq!(settings.encryption, SmtpEncryption::Starttls);
        assert_eq!(settings.username, "mailer");
        assert_eq!(settings.from_email, "no-reply@example.com");
        assert_eq!(settings.from_name, "Example");
        assert_eq!(
            payload.password_change(),
            Some(PasswordChange::Set(PASSWORD))
        );
    }

    #[test]
    fn a_null_smtp_is_a_removal() {
        let payload = SmtpPayloadV1::from_value(&removal()).expect("valid");

        assert_eq!(payload.smtp, None);
        assert_eq!(payload.password_change(), Some(PasswordChange::Remove));
    }

    #[test]
    fn a_save_without_a_password_keeps_the_stored_one() {
        let mut value = save();
        value["smtp"]["password"] = Value::Null;
        let nulled = SmtpPayloadV1::from_value(&value).expect("valid");
        value["smtp"]
            .as_object_mut()
            .expect("object")
            .remove("password");
        let absent = SmtpPayloadV1::from_value(&value).expect("valid");

        assert_eq!(nulled.password_change(), None);
        assert_eq!(absent.password_change(), None);
    }

    #[test]
    fn a_missing_smtp_field_is_refused_rather_than_read_as_a_removal() {
        let mut value = removal();
        value.as_object_mut().expect("object").remove("smtp");

        assert!(matches!(
            SmtpPayloadV1::from_value(&value),
            Err(GenesisError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn every_encryption_is_understood() {
        for (spelled, expected) in [
            ("tls", SmtpEncryption::Tls),
            ("starttls", SmtpEncryption::Starttls),
            ("none", SmtpEncryption::None),
        ] {
            let mut value = save();
            value["smtp"]["encryption"] = json!(spelled);

            let payload = SmtpPayloadV1::from_value(&value).expect("valid");

            assert_eq!(payload.smtp.expect("a save").encryption, expected);
        }
    }

    #[test]
    fn an_unknown_encryption_is_refused() {
        for spelled in ["ssl", "TLS", ""] {
            let mut value = save();
            value["smtp"]["encryption"] = json!(spelled);

            assert!(matches!(
                SmtpPayloadV1::from_value(&value),
                Err(GenesisError::InvalidPayload { .. })
            ));
        }
    }

    #[test]
    fn port_zero_and_out_of_range_ports_are_refused() {
        for port in [json!(0), json!(65536), json!(-1), json!("587")] {
            let mut value = save();
            value["smtp"]["port"] = port;

            assert!(matches!(
                SmtpPayloadV1::from_value(&value),
                Err(GenesisError::InvalidPayload { .. })
            ));
        }
    }

    #[test]
    fn an_empty_realm_is_refused() {
        for realm in ["", "  ", "a/b", "two words"] {
            let mut value = save();
            value["realm"] = json!(realm);

            assert!(matches!(
                SmtpPayloadV1::from_value(&value),
                Err(GenesisError::InvalidPayload { .. })
            ));
        }
    }

    #[test]
    fn an_empty_password_is_refused() {
        let mut value = save();
        value["smtp"]["password"] = json!("");

        assert!(SmtpPayloadV1::from_value(&value).is_err());
    }

    #[test]
    fn the_password_is_redacted_by_debug() {
        let payload = SmtpPayloadV1::from_value(&save()).expect("valid");

        let printed = format!("{payload:?}");

        assert!(!printed.contains(PASSWORD));
        assert!(printed.contains("<redacted>"));
    }

    #[test]
    fn a_password_of_the_wrong_type_is_refused_without_echoing_it() {
        let mut value = save();
        value["smtp"]["password"] = json!(123456789);

        let error = SmtpPayloadV1::from_value(&value).expect_err("refused");

        assert!(!error.to_string().contains("123456789"));
    }

    #[test]
    fn a_refusal_never_carries_the_password() {
        let mut value = save();
        value["smtp"]["port"] = json!(0);

        let error = SmtpPayloadV1::from_value(&value).expect_err("refused");

        assert!(!error.to_string().contains(PASSWORD));
    }
}
