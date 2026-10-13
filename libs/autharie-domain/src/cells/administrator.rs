use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

use crate::{
    CoreError, dataplane::credential::SecretString, dataplane::credential_repository::SealedSecret,
    deployments::DeploymentId,
};

pub const ADMINISTRATOR_CREDENTIAL_LIFETIME: Duration = Duration::days(7);
pub const MAX_ADMINISTRATOR_USERNAME_CHARS: usize = 255;

pub struct InitialAdministrator {
    username: String,
    password: SecretString,
}

impl InitialAdministrator {
    pub fn new(username: impl Into<String>, password: SecretString) -> Self {
        Self {
            username: username.into(),
            password,
        }
    }

    pub fn parse(username: &str, password: SecretString) -> Result<Self, String> {
        let username = username.trim();
        if username.is_empty() || username.chars().count() > MAX_ADMINISTRATOR_USERNAME_CHARS {
            return Err("an administrator needs a username".to_string());
        }
        if password.expose().is_empty() {
            return Err("an administrator needs a password".to_string());
        }
        Ok(Self::new(username, password))
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &SecretString {
        &self.password
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdministratorCredential {
    Available,
    Revealed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TenantAdministrator {
    pub username: String,
    pub credential: AdministratorCredential,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedAdministrator {
    pub username: String,
    pub sealed: SealedSecret,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Taken {
    Sealed(SealedAdministrator),
    Revealed,
    Expired,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdministratorCredentialError {
    #[error("the administrator of this realm has not been created yet")]
    NotIssued,
    #[error("the administrator credential was already revealed and is not kept")]
    AlreadyRevealed,
    #[error("the administrator credential expired before it was revealed and is not kept")]
    Expired,
    #[error("only an organisation administrator can reveal the realm administrator credential")]
    NotAnAdministrator,
}

impl AdministratorCredentialError {
    pub fn is_gone(&self) -> bool {
        matches!(self, Self::AlreadyRevealed | Self::Expired)
    }
}

impl Taken {
    pub fn into_sealed(self) -> Result<SealedAdministrator, AdministratorCredentialError> {
        match self {
            Taken::Sealed(sealed) => Ok(sealed),
            Taken::Revealed => Err(AdministratorCredentialError::AlreadyRevealed),
            Taken::Expired => Err(AdministratorCredentialError::Expired),
            Taken::Absent => Err(AdministratorCredentialError::NotIssued),
        }
    }
}

pub trait TenantAdministratorRepository: Send + Sync {
    fn insert_if_absent(
        &self,
        deployment: DeploymentId,
        administrator: &SealedAdministrator,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn take(
        &self,
        deployment: DeploymentId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<Taken, CoreError>> + Send;

    fn describe(
        &self,
        deployment: DeploymentId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<TenantAdministrator>, CoreError>> + Send;

    fn purge_expired(
        &self,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;
}

pub trait AdministratorVault: Send + Sync {
    fn keep(
        &self,
        deployment: DeploymentId,
        administrator: InitialAdministrator,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn reveal(
        &self,
        deployment: DeploymentId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<InitialAdministrator, CoreError>> + Send;
}

pub fn credential_state(
    revealed: bool,
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AdministratorCredential {
    if revealed {
        AdministratorCredential::Revealed
    } else if expires_at <= now {
        AdministratorCredential::Expired
    } else {
        AdministratorCredential::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap()
    }

    #[test]
    fn a_credential_is_available_until_it_is_revealed_or_expires() {
        let later = now() + ADMINISTRATOR_CREDENTIAL_LIFETIME;

        assert_eq!(
            credential_state(false, later, now()),
            AdministratorCredential::Available
        );
        assert_eq!(
            credential_state(true, later, now()),
            AdministratorCredential::Revealed
        );
        assert_eq!(
            credential_state(false, now(), now()),
            AdministratorCredential::Expired
        );
    }

    #[test]
    fn an_administrator_needs_a_username_and_a_password() {
        assert!(InitialAdministrator::parse("  ", SecretString::new("pw")).is_err());
        assert!(InitialAdministrator::parse("admin", SecretString::new("")).is_err());
        assert!(InitialAdministrator::parse(&"a".repeat(256), SecretString::new("pw")).is_err());

        let parsed = InitialAdministrator::parse(" admin ", SecretString::new("pw")).expect("ok");
        assert_eq!(parsed.username(), "admin");
    }

    #[test]
    fn only_a_revealed_or_expired_credential_is_gone() {
        assert!(AdministratorCredentialError::AlreadyRevealed.is_gone());
        assert!(AdministratorCredentialError::Expired.is_gone());
        assert!(!AdministratorCredentialError::NotIssued.is_gone());
        assert!(!AdministratorCredentialError::NotAnAdministrator.is_gone());
    }

    #[test]
    fn what_was_taken_says_whether_there_is_anything_to_open() {
        assert_eq!(
            Taken::Revealed.into_sealed(),
            Err(AdministratorCredentialError::AlreadyRevealed)
        );
        assert_eq!(
            Taken::Absent.into_sealed(),
            Err(AdministratorCredentialError::NotIssued)
        );
        assert_eq!(
            Taken::Expired.into_sealed(),
            Err(AdministratorCredentialError::Expired)
        );
    }
}
