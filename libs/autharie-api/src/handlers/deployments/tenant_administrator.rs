use autharie_auth::Identity;
use axum::{
    extract::{Extension, State},
    http::header::{CACHE_CONTROL, PRAGMA},
    response::IntoResponse,
};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/administrator/reveal")]
pub struct RevealAdministratorRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct RevealedAdministrator {
    pub username: String,
    /// Temporary: the realm asks for a new password at the first sign-in.
    pub password: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct RevealAdministratorResponse {
    data: RevealedAdministrator,
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/administrator/reveal",
    summary = "reveal the first administrator of a free deployment's realm, once",
    tag = "deployments",
    description = "Returns the username and temporary password of the realm's first \
                   administrator and forgets the password. Every later call answers 410.",
    params(RevealAdministratorRoute),
    responses(
        (status = 200, description = "The credential, shown this once", body = RevealAdministratorResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Not an organisation administrator", body = ApiError),
        (status = 404, description = "No administrator yet", body = ApiError),
        (status = 410, description = "Already revealed, or expired", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn reveal_administrator_handler(
    RevealAdministratorRoute {
        organisation_id,
        deployment_id,
    }: RevealAdministratorRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<impl IntoResponse, ApiError> {
    let administrator = state
        .service
        .reveal_tenant_administrator(identity, organisation_id.into(), deployment_id.into())
        .await?;

    Ok((
        [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
        Response::OK(RevealAdministratorResponse {
            data: RevealedAdministrator {
                username: administrator.username().to_string(),
                password: administrator.password().expose().to_string(),
            },
        }),
    ))
}
