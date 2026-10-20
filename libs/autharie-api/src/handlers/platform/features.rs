use autharie_auth::Identity;
use autharie_core::{
    features::{Feature, FeatureOverview, OrganisationFeature, ports::FeatureFlagService},
    organisation::OrganisationId,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath)]
#[typed_path("/platform/features")]
pub struct FeaturesRoute;

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/features/{key}")]
pub struct FeatureRoute {
    pub key: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/features")]
pub struct TenantFeaturesRoute {
    pub organisation_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/features/{key}")]
pub struct TenantFeatureRoute {
    pub organisation_id: Uuid,
    pub key: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct FeatureView {
    pub key: Feature,
    pub description: String,
    pub enabled_by_default: bool,
    pub overrides_on: usize,
    pub overrides_off: usize,
}

impl From<FeatureOverview> for FeatureView {
    fn from(overview: FeatureOverview) -> Self {
        Self {
            key: overview.feature,
            description: overview.feature.description().to_string(),
            enabled_by_default: overview.enabled_by_default,
            overrides_on: overview.overrides.on,
            overrides_off: overview.overrides.off,
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantFeatureView {
    pub key: Feature,
    pub description: String,
    pub default: bool,
    #[serde(rename = "override")]
    pub organisation_override: Option<bool>,
    pub effective: bool,
}

impl From<OrganisationFeature> for TenantFeatureView {
    fn from(view: OrganisationFeature) -> Self {
        Self {
            key: view.feature,
            description: view.feature.description().to_string(),
            default: view.enabled_by_default,
            organisation_override: view.organisation_override,
            effective: view.effective,
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct FeaturesResponse {
    pub data: Vec<FeatureView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct FeatureResponse {
    pub data: FeatureView,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantFeaturesResponse {
    pub data: Vec<TenantFeatureView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantFeatureResponse {
    pub data: TenantFeatureView,
}

#[derive(Deserialize, ToSchema)]
pub struct SetFeatureDefaultRequest {
    pub enabled_by_default: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetTenantFeatureRequest {
    #[serde(deserialize_with = "Option::deserialize")]
    pub enabled: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/features",
    summary = "list the feature flags",
    tag = "platform",
    description = "Every flag the code defines, with its installation-wide default and how many \
                   organisations deviate from it.",
    responses(
        (status = 200, description = "Every feature flag", body = FeaturesResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_platform_features_handler(
    _: FeaturesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<FeaturesResponse>, ApiError> {
    let features = state.service.list_features(identity).await?;

    Ok(Response::OK(FeaturesResponse {
        data: features.into_iter().map(FeatureView::from).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/features/{key}",
    summary = "set the default of a feature flag",
    tag = "platform",
    description = "Applies to every organisation without an override of its own.",
    params(FeatureRoute),
    request_body = SetFeatureDefaultRequest,
    responses(
        (status = 200, description = "The flag, with its new default", body = FeatureResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the operate_fleet right", body = ApiError),
        (status = 404, description = "No such feature", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_feature_default_handler(
    FeatureRoute { key }: FeatureRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<SetFeatureDefaultRequest>,
) -> Result<Response<FeatureResponse>, ApiError> {
    let feature = key.parse::<Feature>()?;

    let overview = state
        .service
        .set_feature_default(identity, feature, request.enabled_by_default)
        .await?;

    Ok(Response::OK(FeatureResponse {
        data: overview.into(),
    }))
}

#[utoipa::path(
    get,
    path = "/organisations/{organisation_id}/features",
    summary = "list the feature flags of an organisation",
    tag = "platform",
    description = "Each flag with the installation default, the organisation's own override \
                   when it has one, and the value that applies.",
    params(TenantFeaturesRoute),
    responses(
        (status = 200, description = "Every feature flag as this organisation sees it", body = TenantFeaturesResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_tenant_features_handler(
    TenantFeaturesRoute { organisation_id }: TenantFeaturesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<TenantFeaturesResponse>, ApiError> {
    let features = state
        .service
        .list_organisation_features(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(TenantFeaturesResponse {
        data: features.into_iter().map(TenantFeatureView::from).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/organisations/{organisation_id}/features/{key}",
    summary = "set or clear a feature flag for an organisation",
    tag = "platform",
    description = "`true` or `false` overrides the installation default for this organisation; \
                   `null` clears the override.",
    params(TenantFeatureRoute),
    request_body = SetTenantFeatureRequest,
    responses(
        (status = 200, description = "The flag as this organisation now sees it", body = TenantFeatureResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the act_on_tenant right", body = ApiError),
        (status = 404, description = "No such organisation or feature", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_tenant_feature_handler(
    TenantFeatureRoute {
        organisation_id,
        key,
    }: TenantFeatureRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<SetTenantFeatureRequest>,
) -> Result<Response<TenantFeatureResponse>, ApiError> {
    let feature = key.parse::<Feature>()?;

    let view = state
        .service
        .set_organisation_feature(
            identity,
            OrganisationId(organisation_id),
            feature,
            request.enabled,
        )
        .await?;

    Ok(Response::OK(TenantFeatureResponse { data: view.into() }))
}
