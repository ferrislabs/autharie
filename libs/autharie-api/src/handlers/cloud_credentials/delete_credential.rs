use autharie_auth::Identity;
use autharie_core::{
    cloud_credentials::CloudProviderService, dataplane::credential::CloudCredentialId,
};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/cloud-credentials/{credential_id}")]
pub struct CloudCredentialRoute {
    pub organisation_id: Uuid,
    pub credential_id: Uuid,
}

#[derive(Serialize, PartialEq)]
pub struct Empty {}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/cloud-credentials/{credential_id}",
    summary = "delete a cloud credential",
    tag = "cloud-credentials",
    description = "Deletes a registered provider key. Refused while a deployment or a cluster \
                   still uses it.",
    params(CloudCredentialRoute),
    responses(
        (status = 204, description = "Credential deleted"),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 404, description = "Credential not found", body = ApiError),
        (status = 409, description = "Credential in use", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn delete_cloud_credential_handler(
    CloudCredentialRoute {
        organisation_id,
        credential_id,
    }: CloudCredentialRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<Empty>, ApiError> {
    state
        .service
        .delete_cloud_credential(
            identity,
            organisation_id.into(),
            CloudCredentialId(credential_id),
        )
        .await?;

    Ok(Response::NoContent)
}
