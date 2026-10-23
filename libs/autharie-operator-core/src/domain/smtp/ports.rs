use std::collections::BTreeMap;
use std::future::Future;

use autharie_crds::v1alpha::identity_instance::{IdentityInstance, SmtpStatus};

use super::{SmtpError, SmtpWrite};
use crate::domain::OperatorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpOutcome {
    Settled,
    /// Something is waiting on FerrisKey or the cluster; look again soon.
    Retry,
}

pub trait IdentityInstanceSmtp: Send + Sync {
    fn reconcile(&self, instance: &IdentityInstance) -> impl Future<Output = SmtpOutcome> + Send;
}

/// FerrisKey's admin API, as far as a realm's mail provider goes. Every call
/// is made as the instance's admin.
pub trait FerriskeySmtpPort: Send + Sync {
    fn put_config(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        write: &SmtpWrite<'_>,
    ) -> impl Future<Output = Result<(), SmtpError>> + Send;

    /// A realm without a mail provider is already deleted.
    fn delete_config(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> impl Future<Output = Result<(), SmtpError>> + Send;
}

/// Passwords by realm name.
#[derive(Default)]
pub struct SmtpSecrets {
    /// What Genesis wrote to `<instance>-smtp`.
    pub stored: BTreeMap<String, String>,
    /// What the operator last gave FerrisKey, kept in `<instance>-smtp-applied`
    /// because FerrisKey never gives a password back.
    pub applied: BTreeMap<String, String>,
}

pub trait SmtpSecretsPort: Send + Sync {
    fn load(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<SmtpSecrets, SmtpError>> + Send;

    fn put_applied(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        password: &str,
    ) -> impl Future<Output = Result<(), SmtpError>> + Send;

    fn forget_applied(
        &self,
        instance: &IdentityInstance,
        realms: &[&str],
    ) -> impl Future<Output = Result<(), SmtpError>> + Send;
}

/// Writes only `status.smtp`, so it cannot undo what another pass wrote.
pub trait SmtpStatusRepository: Send + Sync {
    fn patch_smtp_status(
        &self,
        instance: &IdentityInstance,
        statuses: &[SmtpStatus],
    ) -> impl Future<Output = Result<(), OperatorError>> + Send;
}
