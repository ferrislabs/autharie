use autharie_auth::Identity;
use autharie_core::dataplane::{ports::DataPlaneService, value_objects::DataPlaneId};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/dataplanes/{dataplane_id}")]
pub struct DeleteDataPlaneRoute {
    pub dataplane_id: DataPlaneId,
}

#[utoipa::path(
    delete,
    path = "/{dataplane_id}",
    summary = "remove a data plane that is out of service",
    tag = "dataplanes",
    description = "Forgets a data plane and revokes the identity its Herald used. Only a disabled \
                   or failed one can go, and only once nothing live is hosted on it: the \
                   deployments already deleted are removed with it. For a cluster in a \
                   customer's cloud account the infrastructure has to be released first, which \
                   happens by itself once the plane is disabled and its deployment is deleted.",
    params(DeleteDataPlaneRoute),
    responses(
        (status = 204, description = "The data plane is gone"),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Changing the fleet is a separate right", body = ApiError),
        (status = 404, description = "No such data plane", body = ApiError),
        (status = 409, description = "The data plane is in service, hosts a live deployment, or still has infrastructure to release", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_dataplane_handler(
    DeleteDataPlaneRoute { dataplane_id }: DeleteDataPlaneRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<()>, ApiError> {
    state
        .service
        .delete_dataplane(identity, dataplane_id)
        .await?;

    Ok(Response::NoContent)
}
