use std::future::Future;

use autharie_crds::v1alpha::identity_instance::{IdentityInstance, PortalThemeStatus};
use serde_json::Value;

use super::PortalThemeError;
use crate::domain::OperatorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalThemeOutcome {
    Settled,
    Retry,
}

pub trait IdentityInstancePortalThemes: Send + Sync {
    fn reconcile(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = PortalThemeOutcome> + Send;
}

pub trait FerriskeyPortalThemePort: Send + Sync {
    fn put_theme(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        config: &Value,
    ) -> impl Future<Output = Result<(), PortalThemeError>> + Send;
}

pub trait PortalThemeStatusRepository: Send + Sync {
    fn patch_portal_theme_status(
        &self,
        instance: &IdentityInstance,
        statuses: &[PortalThemeStatus],
    ) -> impl Future<Output = Result<(), OperatorError>> + Send;
}
