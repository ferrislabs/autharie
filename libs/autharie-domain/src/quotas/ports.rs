use std::{collections::HashMap, future::Future};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    deployments::DeploymentId,
    organisation::OrganisationId,
    quotas::{EffectiveQuota, OrganisationQuota, Quota, QuotaKind},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaChange {
    pub by: String,
    pub at: DateTime<Utc>,
}

/// The overrides platform staff have set. Defaults are not stored: they live
/// in [`crate::quotas::default_quota`] and change by pull request.
pub trait QuotaRepository: Send + Sync {
    fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<HashMap<QuotaKind, Quota>, CoreError>> + Send;

    /// `None` removes the override, which returns the organisation to the
    /// default of each deployment's offer.
    fn set_override(
        &self,
        organisation_id: OrganisationId,
        kind: QuotaKind,
        quota: Option<Quota>,
        change: &QuotaChange,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait OrganisationQuotaService: Send + Sync {
    fn list_organisation_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<OrganisationQuota>, CoreError>> + Send;

    fn set_organisation_quota(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        kind: QuotaKind,
        quota: Option<Quota>,
    ) -> impl Future<Output = Result<OrganisationQuota, CoreError>> + Send;
}

pub trait DeploymentQuotaService: Send + Sync {
    fn deployment_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<EffectiveQuota>, CoreError>> + Send;
}
