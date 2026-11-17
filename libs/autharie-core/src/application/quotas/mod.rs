use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    deployments::DeploymentId,
    organisation::OrganisationId,
    quotas::{
        EffectiveQuota, OrganisationQuota, Quota, QuotaKind,
        ports::{DeploymentQuotaService, OrganisationQuotaService},
        service::{DeploymentQuotaServiceImpl, OrganisationQuotaServiceImpl},
    },
};
use autharie_macros::transactional;

use crate::{
    AutharieService,
    infrastructure::role::permissions_in,
    policy::{AuthariePolicy, PlatformRightsPolicy},
};

impl OrganisationQuotaService for AutharieService {
    #[transactional(quota, platform_operator, organisation, audit, user)]
    async fn list_organisation_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationQuota>, CoreError> {
        OrganisationQuotaServiceImpl::new(
            quota_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
        )
        .list_organisation_quotas(identity, organisation_id)
        .await
    }

    #[transactional(quota, platform_operator, organisation, audit, user)]
    async fn set_organisation_quota(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        kind: QuotaKind,
        quota: Option<Quota>,
    ) -> Result<OrganisationQuota, CoreError> {
        OrganisationQuotaServiceImpl::new(
            quota_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
        )
        .set_organisation_quota(identity, organisation_id, kind, quota)
        .await
    }
}

impl DeploymentQuotaService for AutharieService {
    #[transactional(quota, deployment)]
    async fn deployment_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<EffectiveQuota>, CoreError> {
        DeploymentQuotaServiceImpl::new(
            quota_repository,
            deployment_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .deployment_quotas(identity, organisation_id, deployment_id)
        .await
    }
}
