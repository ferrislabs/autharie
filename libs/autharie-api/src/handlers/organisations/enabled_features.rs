use autharie_auth::Identity;
use autharie_core::{
    features::{Feature, ports::OrganisationFeatureService},
    organisation::OrganisationId,
};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/feature-flags")]
pub struct FeatureFlagsRoute {
    pub organisation_id: Uuid,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct EnabledFeaturesResponse {
    pub enabled: Vec<Feature>,
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/feature-flags",
    summary = "list the feature flags enabled for an organisation",
    tag = "organisations",
    description = "The keys that apply to this organisation after the installation default and \
                   the organisation's own override are resolved. A flag that is off is absent.",
    params(FeatureFlagsRoute),
    responses(
        (status = 200, description = "The enabled feature keys", body = EnabledFeaturesResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller is not in this organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_enabled_features_handler(
    FeatureFlagsRoute { organisation_id }: FeatureFlagsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<EnabledFeaturesResponse>, ApiError> {
    let enabled = state
        .service
        .enabled_features(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(EnabledFeaturesResponse { enabled }))
}
