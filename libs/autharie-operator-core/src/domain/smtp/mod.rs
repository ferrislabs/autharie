pub mod plan;
pub mod ports;
pub mod service;

use std::fmt;

use autharie_crds::v1alpha::identity_instance::SmtpEncryption;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtpError {
    /// Worth trying again as is: FerrisKey or the cluster was down, slow or busy.
    Retry { message: String },
    /// FerrisKey said no; the same call will say it again.
    Rejected { message: String },
}

impl SmtpError {
    pub fn message(&self) -> &str {
        match self {
            Self::Retry { message } | Self::Rejected { message } => message,
        }
    }

    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
}

/// What FerrisKey is given for a realm. It wants the password with every
/// write and never returns it.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpWrite<'a> {
    pub host: &'a str,
    pub port: u16,
    pub encryption: SmtpEncryption,
    pub username: &'a str,
    pub password: &'a str,
    pub from_email: &'a str,
    pub from_name: &'a str,
}

impl fmt::Debug for SmtpWrite<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpWrite")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("encryption", &self.encryption)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("from_email", &self.from_email)
            .field("from_name", &self.from_name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_never_reaches_a_log_line() {
        let write = SmtpWrite {
            host: "smtp.example.com",
            port: 587,
            encryption: SmtpEncryption::Starttls,
            username: "mailer",
            password: "hunter2-secret",
            from_email: "no-reply@example.com",
            from_name: "Acme",
        };

        let shown = format!("{write:?}");

        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("smtp.example.com"));
    }

    #[test]
    fn only_a_retry_is_worth_retrying() {
        assert!(
            SmtpError::Retry {
                message: "x".to_string()
            }
            .is_retry()
        );
        assert!(
            !SmtpError::Rejected {
                message: "x".to_string()
            }
            .is_retry()
        );
    }
}
