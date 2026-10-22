use autharie_auth::Identity;
use autharie_core::{cloud_credentials::CloudProviderService, dataplane::cloud_provider::Provider};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::Serialize;
use utoipa::ToSchema;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ListCloudProvidersResponse {
    pub data: Vec<Provider>,
}

#[derive(TypedPath)]
#[typed_path("/cloud-providers")]
pub struct ListCloudProvidersRoute;

#[utoipa::path(
    get,
    path = "",
    summary = "list the cloud providers this installation can connect",
    tag = "cloud-providers",
    description = "The providers an organisation can register a credential for here. Empty when \
                   customer cloud is off, or on but unable to run, so a client can tell whether \
                   to offer cloud accounts at all. Says nothing about any organisation, which is \
                   what makes it safe to answer for any caller.",
    responses(
        (status = 200, description = "Providers", body = ListCloudProvidersResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_cloud_providers_handler(
    _: ListCloudProvidersRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<ListCloudProvidersResponse>, ApiError> {
    let providers = state.service.list_cloud_providers(identity).await?;

    Ok(Response::OK(ListCloudProvidersResponse { data: providers }))
}
