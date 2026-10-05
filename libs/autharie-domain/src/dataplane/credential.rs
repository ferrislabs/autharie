use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{dataplane::cloud_provider::Provider, organisation::OrganisationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct CloudCredentialId(pub Uuid);

impl FromStr for CloudCredentialId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::from_str(s).map(CloudCredentialId)
    }
}

impl fmt::Display for CloudCredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A provider key in the clear.
///
/// No `Debug`, no `Display`, no `Serialize`, and zeroed when it goes out of
/// scope. Every one of those is a way a key reaches a log, and a log is the
/// one place it can never be taken back from.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// The outcome of the permission check a credential passed to be registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct ScopeCheck {
    pub checked_at: DateTime<Utc>,
}

/// Everything about a credential except the secret, which only the store holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CloudCredential {
    pub id: CloudCredentialId,
    pub organisation_id: OrganisationId,
    pub provider: Provider,
    pub label: String,
    pub scope_check: ScopeCheck,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("the credential is not valid for this provider")]
    Invalid,

    #[error("the credential lacks required permissions: {}", missing.join(", "))]
    MissingPermissions { missing: Vec<String> },

    #[error("the credential holds more permissions than needed: {}", extra.join(", "))]
    ExcessPermissions { extra: Vec<String> },

    #[error("the credential is used by a deployment and cannot be deleted")]
    InUse,

    #[error("credential {id} does not exist")]
    NotFound { id: CloudCredentialId },

    #[error("the credential store failed: {0}")]
    Store(String),

    #[error("customer cloud is not enabled on this installation")]
    NotEnabled,
}

/// Holds provider secrets. The adapter wraps them with the transit key; a
/// secret never leaves it except through `get_for_provisioning`.
#[cfg_attr(test, mockall::automock)]
pub trait CloudCredentialStore: Send + Sync {
    fn put(
        &self,
        organisation_id: OrganisationId,
        provider: Provider,
        secret: SecretString,
    ) -> impl Future<Output = Result<CloudCredentialId, CredentialError>> + Send;

    fn get_for_provisioning(
        &self,
        id: &CloudCredentialId,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    fn delete(
        &self,
        id: &CloudCredentialId,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send;
}
