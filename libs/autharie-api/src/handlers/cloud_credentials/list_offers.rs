use autharie_auth::Identity;
use autharie_core::{
    cloud_credentials::CloudProviderService,
    dataplane::{
        cloud_provider::ProviderOffers, credential::CloudCredentialId, value_objects::Region,
    },
};
use axum::{
    Extension,
    extract::{Query, State},
};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/cloud-credentials/{credential_id}/offers")]
pub struct ProviderOffersRoute {
    pub organisation_id: Uuid,
    pub credential_id: Uuid,
}

#[derive(Deserialize, IntoParams)]
pub struct ProviderOffersQuery {
    pub region: String,
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/cloud-credentials/{credential_id}/offers",
    summary = "list what the provider sells in a region",
    tag = "cloud-credentials",
    description = "Reads the provider's catalog for a region with the credential: control \
                   planes and node types, with monthly prices in minor units.",
    params(ProviderOffersRoute, ProviderOffersQuery),
    responses(
        (status = 200, description = "The offers", body = ProviderOffers),
        (status = 400, description = "Invalid request data", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 404, description = "Credential not found", body = ApiError),
        (status = 502, description = "The provider refused or could not serve the request", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_provider_offers_handler(
    ProviderOffersRoute {
        organisation_id,
        credential_id,
    }: ProviderOffersRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<ProviderOffersQuery>,
) -> Result<Response<ProviderOffers>, ApiError> {
    if query.region.trim().is_empty() {
        return Err(ApiError::BadRequest {
            reason: "a region is required".to_string(),
        });
    }

    let offers = state
        .service
        .list_provider_offers(
            identity,
            organisation_id.into(),
            CloudCredentialId(credential_id),
            Region::new(query.region),
        )
        .await?;

    Ok(Response::OK(offers))
}
