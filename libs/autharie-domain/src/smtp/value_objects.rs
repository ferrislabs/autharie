use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::SmtpError;
use crate::{dataplane::credential::SecretString, webhooks::WebhookRealm};

pub const MAX_HOST_LENGTH: usize = 253;
pub const MAX_USERNAME_LENGTH: usize = 255;
pub const MAX_EMAIL_LENGTH: usize = 254;
pub const MAX_EMAIL_LOCAL_LENGTH: usize = 64;
pub const MAX_FROM_NAME_LENGTH: usize = 255;
pub const MAX_PASSWORD_LENGTH: usize = 1024;

pub type SmtpRealm = WebhookRealm;

pub fn parse_realm(value: &str) -> Result<SmtpRealm, SmtpError> {
    WebhookRealm::parse(value).map_err(|error| SmtpError::invalid(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "smtp.example.com")]
pub struct SmtpHost(String);

impl SmtpHost {
    pub fn parse(value: &str) -> Result<Self, SmtpError> {
        let value = value.trim();

        if value.is_empty() || value.len() > MAX_HOST_LENGTH {
            return Err(SmtpError::invalid(format!(
                "the host must hold between 1 and {MAX_HOST_LENGTH} characters"
            )));
        }

        if value.contains("://") || value.contains('/') {
            return Err(SmtpError::invalid(
                "the host is a name, not a URL: leave out the scheme and any path",
            ));
        }

        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
        {
            return Err(SmtpError::invalid(
                "the host may hold only letters, digits, hyphens, dots and underscores",
            ));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = u16, example = 587)]
pub struct SmtpPort(u16);

impl SmtpPort {
    pub fn parse(value: i64) -> Result<Self, SmtpError> {
        u16::try_from(value)
            .ok()
            .filter(|port| *port >= 1)
            .map(Self)
            .ok_or_else(|| SmtpError::invalid("the port must be between 1 and 65535"))
    }

    pub fn value(&self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmtpEncryption {
    Tls,
    Starttls,
    None,
}

impl SmtpEncryption {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::Starttls => "starttls",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SmtpError> {
        match value {
            "tls" => Ok(Self::Tls),
            "starttls" => Ok(Self::Starttls),
            "none" => Ok(Self::None),
            other => Err(SmtpError::invalid(format!(
                "'{other}' is not an encryption: use tls, starttls or none"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct SmtpUsername(String);

impl SmtpUsername {
    pub fn parse(value: &str) -> Result<Self, SmtpError> {
        let length = value.chars().count();

        if value.trim().is_empty() || length > MAX_USERNAME_LENGTH {
            return Err(SmtpError::invalid(format!(
                "the username must hold between 1 and {MAX_USERNAME_LENGTH} characters"
            )));
        }

        if value.chars().any(char::is_control) {
            return Err(SmtpError::invalid(
                "the username may not hold control characters",
            ));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "no-reply@example.com")]
pub struct FromEmail(String);

impl FromEmail {
    pub fn parse(value: &str) -> Result<Self, SmtpError> {
        let value = value.trim();
        let invalid = || SmtpError::invalid("the from address is not a valid email address");

        if value.len() > MAX_EMAIL_LENGTH
            || value.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(invalid());
        }

        let (local, domain) = value.split_once('@').ok_or_else(invalid)?;

        let local_valid = !local.is_empty()
            && local.len() <= MAX_EMAIL_LOCAL_LENGTH
            && !local.starts_with('.')
            && !local.ends_with('.')
            && !local.contains("..")
            && local
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~.-".contains(c));

        let domain_valid = !domain.is_empty()
            && domain.split('.').all(|label| {
                !label.is_empty()
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });

        if !local_valid || !domain_valid {
            return Err(invalid());
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct FromName(String);

impl FromName {
    pub fn parse(value: &str) -> Result<Self, SmtpError> {
        let value = value.trim();

        if value.chars().count() > MAX_FROM_NAME_LENGTH {
            return Err(SmtpError::invalid(format!(
                "the from name may hold at most {MAX_FROM_NAME_LENGTH} characters"
            )));
        }

        if value.chars().any(char::is_control) {
            return Err(SmtpError::invalid(
                "the from name may not hold control characters",
            ));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub struct SmtpPassword(SecretString);

impl SmtpPassword {
    pub fn parse(value: &str) -> Result<Self, SmtpError> {
        if value.is_empty() || value.chars().count() > MAX_PASSWORD_LENGTH {
            return Err(SmtpError::invalid(format!(
                "the password must hold between 1 and {MAX_PASSWORD_LENGTH} characters"
            )));
        }

        Ok(Self(SecretString::new(value)))
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }

    pub fn secret(&self) -> &SecretString {
        &self.0
    }
}

impl fmt::Debug for SmtpPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SmtpPassword(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_a_plain_name() {
        assert!(SmtpHost::parse("smtp.example.com").is_ok());
        assert!(SmtpHost::parse("10.0.0.7").is_ok());
        assert!(SmtpHost::parse("mail_relay.internal").is_ok());
    }

    #[test]
    fn a_host_with_a_scheme_a_slash_or_a_space_is_refused() {
        for host in [
            "",
            "   ",
            "smtp://mail.example.com",
            "mail.example.com/relay",
            "mail example.com",
            "mail.exa\nmple.com",
            "mail.example.com:587",
        ] {
            assert!(SmtpHost::parse(host).is_err(), "{host:?}");
        }
    }

    #[test]
    fn a_host_is_at_most_253_characters() {
        assert!(SmtpHost::parse(&"a".repeat(253)).is_ok());
        assert!(SmtpHost::parse(&"a".repeat(254)).is_err());
    }

    #[test]
    fn a_port_is_between_1_and_65535() {
        assert_eq!(SmtpPort::parse(587).unwrap().value(), 587);
        assert_eq!(SmtpPort::parse(1).unwrap().value(), 1);
        assert_eq!(SmtpPort::parse(65535).unwrap().value(), 65535);

        for port in [0, -1, 65536, 70000] {
            assert!(SmtpPort::parse(port).is_err(), "{port}");
        }
    }

    #[test]
    fn an_encryption_reads_its_wire_name() {
        assert_eq!(SmtpEncryption::parse("tls"), Ok(SmtpEncryption::Tls));
        assert_eq!(
            SmtpEncryption::parse("starttls"),
            Ok(SmtpEncryption::Starttls)
        );
        assert_eq!(SmtpEncryption::parse("none"), Ok(SmtpEncryption::None));
        assert!(SmtpEncryption::parse("TLS").is_err());
        assert!(SmtpEncryption::parse("ssl").is_err());
    }

    #[test]
    fn an_encryption_serialises_in_lowercase() {
        assert_eq!(
            serde_json::to_value(SmtpEncryption::Starttls).unwrap(),
            serde_json::json!("starttls")
        );
    }

    #[test]
    fn a_username_is_1_to_255_characters() {
        assert!(SmtpUsername::parse("apikey").is_ok());
        assert!(SmtpUsername::parse("").is_err());
        assert!(SmtpUsername::parse("  ").is_err());
        assert!(SmtpUsername::parse(&"u".repeat(255)).is_ok());
        assert!(SmtpUsername::parse(&"u".repeat(256)).is_err());
    }

    #[test]
    fn a_from_address_needs_a_local_part_and_a_domain() {
        assert!(FromEmail::parse("no-reply@example.com").is_ok());
        assert!(FromEmail::parse("first.last+tag@mail.example.com").is_ok());

        for address in [
            "",
            "example.com",
            "@example.com",
            "no-reply@",
            "a@b@example.com",
            "no reply@example.com",
            "no-reply@exa mple.com",
            "no-reply@example..com",
            ".a@example.com",
            "no-reply@-example.com",
        ] {
            assert!(FromEmail::parse(address).is_err(), "{address:?}");
        }
    }

    #[test]
    fn a_from_address_is_at_most_254_characters() {
        let long = format!("a@{}.com", "d".repeat(260));

        assert!(FromEmail::parse(&long).is_err());
    }

    #[test]
    fn a_from_name_may_be_empty_but_not_long() {
        assert_eq!(FromName::parse("").unwrap().as_str(), "");
        assert!(FromName::parse(&"n".repeat(255)).is_ok());
        assert!(FromName::parse(&"n".repeat(256)).is_err());
    }

    #[test]
    fn a_password_is_1_to_1024_characters() {
        assert!(SmtpPassword::parse("x").is_ok());
        assert!(SmtpPassword::parse("").is_err());
        assert!(SmtpPassword::parse(&"p".repeat(1024)).is_ok());
        assert!(SmtpPassword::parse(&"p".repeat(1025)).is_err());
    }

    #[test]
    fn a_password_does_not_print() {
        let password = SmtpPassword::parse("hunter2-canary").unwrap();

        assert!(!format!("{password:?}").contains("hunter2"));
        assert!(!format!("{password:#?}").contains("hunter2"));
    }

    #[test]
    fn a_realm_reuses_the_webhook_rules() {
        assert!(parse_realm("master").is_ok());
        assert!(parse_realm("my-realm_2").is_ok());
        assert!(parse_realm("bad realm").is_err());
        assert!(parse_realm("").is_err());
    }
}
