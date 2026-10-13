use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

use crate::{
    CoreError, dataplane::credential::SecretString, dataplane::credential_repository::SealedSecret,
    deployments::DeploymentId,
};

pub const ADMINISTRATOR_CREDENTIAL_LIFETIME: Duration = Duration::days(7);
pub const ADMINISTRATOR_ISSUE_STALE_AFTER: Duration = Duration::minutes(10);
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
    Issuing,
    Ready,
    Revealed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TenantAdministrator {
    pub credential: AdministratorCredential,
    pub stale: bool,
    pub username: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueRecord {
    pub requested_at: DateTime<Utc>,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revealed: bool,
}

impl IssueRecord {
    pub fn state(&self, now: DateTime<Utc>) -> (AdministratorCredential, bool) {
        let credential = match (self.issued_at, self.expires_at) {
            (None, _) | (_, None) => AdministratorCredential::Issuing,
            _ if self.revealed => AdministratorCredential::Revealed,
            (_, Some(expires_at)) if expires_at <= now => AdministratorCredential::Expired,
            _ => AdministratorCredential::Ready,
        };
        let stale = credential == AdministratorCredential::Issuing
            && now - self.requested_at > ADMINISTRATOR_ISSUE_STALE_AFTER;
        (credential, stale)
    }

    pub fn describe(&self, username: Option<String>, now: DateTime<Utc>) -> TenantAdministrator {
        let (credential, stale) = self.state(now);
        TenantAdministrator {
            credential,
            stale,
            username,
            requested_at: self.requested_at,
            expires_at: self.expires_at,
        }
    }

    pub fn may_issue_again(&self, now: DateTime<Utc>) -> bool {
        !matches!(self.state(now), (AdministratorCredential::Issuing, false))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedAdministrator {
    pub username: String,
    pub sealed: SealedSecret,
    pub issued_at: DateTime<Utc>,
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
    #[error("no administrator credential is ready to reveal: generate one first")]
    NotIssued,
    #[error("an administrator credential is already being generated")]
    AlreadyIssuing,
    #[error("the administrator credential was already revealed and is not kept")]
    AlreadyRevealed,
    #[error("the administrator credential expired before it was revealed and is not kept")]
    Expired,
    #[error("only an organisation administrator can manage the realm administrator credential")]
    NotAnAdministrator,
    #[error("this deployment has no realm administrator to manage")]
    NotARealm,
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
    fn issue_record(
        &self,
        deployment: DeploymentId,
    ) -> impl Future<Output = Result<Option<IssueRecord>, CoreError>> + Send;

    fn request_issue(
        &self,
        deployment: DeploymentId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn store_issued(
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap()
    }

    fn issued(revealed: bool, expires_at: DateTime<Utc>) -> IssueRecord {
        IssueRecord {
            requested_at: now() - Duration::hours(1),
            issued_at: Some(now() - Duration::minutes(59)),
            expires_at: Some(expires_at),
            revealed,
        }
    }

    #[test]
    fn a_requested_credential_is_issuing_until_it_arrives_and_stale_after_ten_minutes() {
        let fresh = IssueRecord {
            requested_at: now() - Duration::minutes(9),
            issued_at: None,
            expires_at: None,
            revealed: false,
        };
        let old = IssueRecord {
            requested_at: now() - Duration::minutes(11),
            ..fresh
        };

        assert_eq!(
            fresh.state(now()),
            (AdministratorCredential::Issuing, false)
        );
        assert_eq!(old.state(now()), (AdministratorCredential::Issuing, true));
        assert!(!fresh.may_issue_again(now()));
        assert!(old.may_issue_again(now()));
    }

    #[test]
    fn an_issued_credential_is_ready_until_revealed_or_expired() {
        let later = now() + ADMINISTRATOR_CREDENTIAL_LIFETIME;

        assert_eq!(
            issued(false, later).state(now()).0,
            AdministratorCredential::Ready
        );
        assert_eq!(
            issued(true, later).state(now()).0,
            AdministratorCredential::Revealed
        );
        assert_eq!(
            issued(false, now()).state(now()).0,
            AdministratorCredential::Expired
        );
        assert!(issued(true, later).may_issue_again(now()));
        assert!(issued(false, later).may_issue_again(now()));
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
        assert!(!AdministratorCredentialError::AlreadyIssuing.is_gone());
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
