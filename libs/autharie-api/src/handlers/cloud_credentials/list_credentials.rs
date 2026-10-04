use autharie_auth::Identity;
use autharie_core::cloud_credentials::CloudProviderService;
use axum::{Extension, extract::State};

use super::register_credential::{CloudCredentialResponse, CloudCredentialsRoute};
use crate::{errors::ApiError, response::Response, state::AppState};

#[utoipa::path(
    get,
    path = "/{organisation_id}/cloud-credentials",
    summary = "list cloud credentials",
    tag = "cloud-credentials",
    description = "Lists the organisation's registered provider keys, without the keys.",
    params(CloudCredentialsRoute),
    responses(
        (status = 200, description = "The credentials", body = Vec<CloudCredentialResponse>),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_cloud_credentials_handler(
    CloudCredentialsRoute { organisation_id }: CloudCredentialsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<Vec<CloudCredentialResponse>>, ApiError> {
    let credentials = state
        .service
        .list_cloud_credentials(identity, organisation_id.into())
        .await?;

    Ok(Response::OK(
        credentials
            .into_iter()
            .map(CloudCredentialResponse::from)
            .collect(),
    ))
}
