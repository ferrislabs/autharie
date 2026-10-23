use std::fmt;

use super::{
    SmtpError,
    entities::{SmtpSettings, SmtpState},
    value_objects::{
        FromEmail, FromName, SmtpEncryption, SmtpHost, SmtpPassword, SmtpPort, SmtpRealm,
        SmtpUsername, parse_realm,
    },
};

#[derive(Clone)]
pub struct RawSmtpInput {
    pub host: String,
    pub port: i64,
    pub encryption: String,
    pub username: String,
    pub password: Option<String>,
    pub from_email: String,
    pub from_name: String,
}

impl fmt::Debug for RawSmtpInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawSmtpInput")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("encryption", &self.encryption)
            .field("username", &self.username)
            .field("from_email", &self.from_email)
            .field("from_name", &self.from_name)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct SmtpInput {
    pub realm: SmtpRealm,
    pub settings: SmtpSettings,
    pub password: Option<SmtpPassword>,
}

impl SmtpInput {
    pub fn parse(realm: &str, raw: RawSmtpInput) -> Result<Self, SmtpError> {
        Ok(Self {
            realm: parse_realm(realm)?,
            settings: SmtpSettings {
                host: SmtpHost::parse(&raw.host)?,
                port: SmtpPort::parse(raw.port)?,
                encryption: SmtpEncryption::parse(&raw.encryption)?,
                username: SmtpUsername::parse(&raw.username)?,
                from_email: FromEmail::parse(&raw.from_email)?,
                from_name: FromName::parse(&raw.from_name)?,
            },
            password: raw
                .password
                .as_deref()
                .map(SmtpPassword::parse)
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpOutcome {
    pub revision: u64,
    pub realm: SmtpRealm,
    pub state: SmtpState,
}

impl SmtpOutcome {
    pub fn parse(
        revision: u64,
        realm: &str,
        items: Vec<(String, Option<String>)>,
    ) -> Result<Self, SmtpError> {
        let [(phase, message)] = <[_; 1]>::try_from(items).map_err(|items: Vec<_>| {
            SmtpError::invalid(format!(
                "an smtp report holds exactly one item, not {}",
                items.len()
            ))
        })?;

        let state = match phase.as_str() {
            "Pending" => SmtpState::Pending,
            "Applied" => SmtpState::Applied,
            "Failed" => SmtpState::Failed {
                message: message.unwrap_or_else(|| "the data plane gave no reason".to_string()),
            },
            other => {
                return Err(SmtpError::invalid(format!(
                    "'{other}' is not an email provider phase"
                )));
            }
        };

        Ok(Self {
            revision,
            realm: parse_realm(realm)?,
            state,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> RawSmtpInput {
        RawSmtpInput {
            host: "smtp.example.com".to_string(),
            port: 587,
            encryption: "starttls".to_string(),
            username: "apikey".to_string(),
            password: Some("pw".to_string()),
            from_email: "no-reply@example.com".to_string(),
            from_name: "Acme".to_string(),
        }
    }

    #[test]
    fn a_valid_input_is_read() {
        let input = SmtpInput::parse("master", raw()).unwrap();

        assert_eq!(input.realm.as_str(), "master");
        assert_eq!(input.settings.port.value(), 587);
        assert_eq!(input.password.unwrap().expose(), "pw");
    }

    #[test]
    fn a_missing_password_is_kept_missing() {
        let mut keep = raw();
        keep.password = None;

        assert!(SmtpInput::parse("master", keep).unwrap().password.is_none());
    }

    #[test]
    fn an_empty_password_is_refused_rather_than_kept() {
        let mut empty = raw();
        empty.password = Some(String::new());

        assert!(SmtpInput::parse("master", empty).is_err());
    }

    #[test]
    fn a_bad_field_refuses_the_whole_input() {
        let bad = |change: fn(&mut RawSmtpInput)| {
            let mut input = raw();
            change(&mut input);
            SmtpInput::parse("master", input).is_err()
        };

        assert!(bad(
            |input| input.host = "smtp://mail.example.com".to_string()
        ));
        assert!(bad(|input| input.port = 0));
        assert!(bad(|input| input.port = 70000));
        assert!(bad(|input| input.encryption = "ssl".to_string()));
        assert!(bad(|input| input.username = String::new()));
        assert!(bad(|input| input.from_email = "nobody".to_string()));
        assert!(bad(|input| input.from_name = "n".repeat(256)));
        assert!(SmtpInput::parse("bad realm", raw()).is_err());
    }

    #[test]
    fn an_input_does_not_print_its_password() {
        let mut with_canary = raw();
        with_canary.password = Some("hunter2-canary".to_string());
        let input = SmtpInput::parse("master", with_canary).unwrap();

        assert!(!format!("{input:?}").contains("hunter2"));
        assert!(!format!("{input:#?}").contains("hunter2"));

        let mut raw_with_canary = raw();
        raw_with_canary.password = Some("hunter2-canary".to_string());
        assert!(!format!("{raw_with_canary:?}").contains("hunter2"));
    }

    #[test]
    fn an_outcome_reads_its_single_item() {
        let applied = SmtpOutcome::parse(3, "master", vec![("Applied".to_string(), None)]).unwrap();
        assert_eq!(applied.state, SmtpState::Applied);
        assert_eq!(applied.revision, 3);

        let failed = SmtpOutcome::parse(
            3,
            "master",
            vec![("Failed".to_string(), Some("refused".to_string()))],
        )
        .unwrap();
        assert_eq!(
            failed.state,
            SmtpState::Failed {
                message: "refused".to_string()
            }
        );

        let pending = SmtpOutcome::parse(3, "master", vec![("Pending".to_string(), None)]).unwrap();
        assert_eq!(pending.state, SmtpState::Pending);
    }

    #[test]
    fn an_outcome_needs_one_item_a_known_phase_and_a_realm() {
        assert!(SmtpOutcome::parse(1, "master", vec![]).is_err());
        assert!(
            SmtpOutcome::parse(
                1,
                "master",
                vec![("Applied".to_string(), None), ("Applied".to_string(), None)]
            )
            .is_err()
        );
        assert!(SmtpOutcome::parse(1, "master", vec![("Exploded".to_string(), None)]).is_err());
        assert!(SmtpOutcome::parse(1, "bad realm", vec![("Applied".to_string(), None)]).is_err());
    }
}
