use std::{collections::HashMap, future::Future};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    features::{Feature, FeatureOverview, OrganisationFeature, OverrideTally, resolve},
    organisation::OrganisationId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagChange {
    pub by: String,
    pub at: DateTime<Utc>,
}

pub trait FeatureRepository: Send + Sync {
    fn defaults(&self) -> impl Future<Output = Result<HashMap<Feature, bool>, CoreError>> + Send;

    fn set_default(
        &self,
        feature: Feature,
        enabled: bool,
        change: &FlagChange,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<HashMap<Feature, bool>, CoreError>> + Send;

    fn set_override(
        &self,
        organisation_id: OrganisationId,
        feature: Feature,
        enabled: Option<bool>,
        change: &FlagChange,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn override_tallies(
        &self,
    ) -> impl Future<Output = Result<HashMap<Feature, OverrideTally>, CoreError>> + Send;

    fn is_enabled(
        &self,
        organisation_id: OrganisationId,
        feature: Feature,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send {
        async move {
            let defaults = self.defaults().await?;
            let overrides = self.overrides(organisation_id).await?;

            Ok(resolve(
                overrides.get(&feature).copied(),
                defaults.get(&feature).copied(),
            ))
        }
    }

    fn enabled_for(
        &self,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<Feature>, CoreError>> + Send {
        async move {
            let defaults = self.defaults().await?;
            let overrides = self.overrides(organisation_id).await?;

            Ok(Feature::ALL
                .into_iter()
                .filter(|feature| {
                    resolve(
                        overrides.get(feature).copied(),
                        defaults.get(feature).copied(),
                    )
                })
                .collect())
        }
    }
}

pub trait FeatureFlagService: Send + Sync {
    fn list_features(
        &self,
        identity: Identity,
    ) -> impl Future<Output = Result<Vec<FeatureOverview>, CoreError>> + Send;

    fn set_feature_default(
        &self,
        identity: Identity,
        feature: Feature,
        enabled: bool,
    ) -> impl Future<Output = Result<FeatureOverview, CoreError>> + Send;

    fn list_organisation_features(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<OrganisationFeature>, CoreError>> + Send;

    fn set_organisation_feature(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        feature: Feature,
        enabled: Option<bool>,
    ) -> impl Future<Output = Result<OrganisationFeature, CoreError>> + Send;
}

pub trait OrganisationFeatureService: Send + Sync {
    fn enabled_features(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<Feature>, CoreError>> + Send;
}
