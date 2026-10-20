use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    features::{
        Feature, FeatureOverview, OrganisationFeature,
        ports::{FeatureFlagService, OrganisationFeatureService},
        service::{FeatureFlagServiceImpl, OrganisationFeatureServiceImpl},
    },
    organisation::OrganisationId,
};
use autharie_macros::transactional;

use crate::{
    AutharieService,
    infrastructure::role::permissions_in,
    policy::{AuthariePolicy, PlatformRightsPolicy},
};

impl FeatureFlagService for AutharieService {
    #[transactional(
        feature_flag,
        platform_operator,
        organisation,
        audit,
        user,
        fleet_audit
    )]
    async fn list_features(&self, identity: Identity) -> Result<Vec<FeatureOverview>, CoreError> {
        FeatureFlagServiceImpl::new(
            feature_flag_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
            fleet_audit_repository,
        )
        .list_features(identity)
        .await
    }

    #[transactional(
        feature_flag,
        platform_operator,
        organisation,
        audit,
        user,
        fleet_audit
    )]
    async fn set_feature_default(
        &self,
        identity: Identity,
        feature: Feature,
        enabled: bool,
    ) -> Result<FeatureOverview, CoreError> {
        FeatureFlagServiceImpl::new(
            feature_flag_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
            fleet_audit_repository,
        )
        .set_feature_default(identity, feature, enabled)
        .await
    }

    #[transactional(
        feature_flag,
        platform_operator,
        organisation,
        audit,
        user,
        fleet_audit
    )]
    async fn list_organisation_features(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationFeature>, CoreError> {
        FeatureFlagServiceImpl::new(
            feature_flag_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
            fleet_audit_repository,
        )
        .list_organisation_features(identity, organisation_id)
        .await
    }

    #[transactional(
        feature_flag,
        platform_operator,
        organisation,
        audit,
        user,
        fleet_audit
    )]
    async fn set_organisation_feature(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        feature: Feature,
        enabled: Option<bool>,
    ) -> Result<OrganisationFeature, CoreError> {
        FeatureFlagServiceImpl::new(
            feature_flag_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
            fleet_audit_repository,
        )
        .set_organisation_feature(identity, organisation_id, feature, enabled)
        .await
    }
}

impl OrganisationFeatureService for AutharieService {
    #[transactional(feature_flag)]
    async fn enabled_features(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<Feature>, CoreError> {
        OrganisationFeatureServiceImpl::new(
            feature_flag_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .enabled_features(identity, organisation_id)
        .await
    }
}
