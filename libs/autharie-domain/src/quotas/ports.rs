use std::{collections::HashMap, future::Future};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    CoreError,
    deployments::DeploymentId,
    organisation::OrganisationId,
    quotas::{
        EffectiveQuota, OrganisationQuota, OrganisationQuotaKind, OrganisationScopeQuota, Quota,
        QuotaKind,
    },
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

    fn organisation_overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<HashMap<OrganisationQuotaKind, Quota>, CoreError>> + Send;

    fn set_organisation_override(
        &self,
        organisation_id: OrganisationId,
        kind: OrganisationQuotaKind,
        quota: Option<Quota>,
        change: &QuotaChange,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Makes the creates of one counted kind under one scope (a deployment, an
    /// organisation) take turns until the transaction ends.
    ///
    /// Called before the count and held across the insert, so two creates at
    /// `max - 1` cannot both read `max - 1`. The lock lives in the adapter; the
    /// domain only says which creates must not overlap.
    fn serialise_creates(
        &self,
        scope: Uuid,
        key: &str,
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

pub trait OrganisationScopeQuotaService: Send + Sync {
    fn list_organisation_scope_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<OrganisationScopeQuota>, CoreError>> + Send;

    fn set_organisation_scope_quota(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        kind: OrganisationQuotaKind,
        quota: Option<Quota>,
    ) -> impl Future<Output = Result<OrganisationScopeQuota, CoreError>> + Send;
}
