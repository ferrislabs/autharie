use autharie_auth::Identity;
use autharie_core::{
    cloud_credentials::CloudProviderService, dataplane::cluster_profile::ClusterProfile,
    deployments::DeploymentId, organisation::OrganisationId,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    errors::ApiError, handlers::cloud_credentials::profile::ClusterProfileRequest,
    response::Response, state::AppState,
};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/cluster-profile")]
pub struct ClusterProfileRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ResizeClusterResponse {
    pub data: ClusterProfile,
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/cluster-profile",
    summary = "resize the cluster of a customer cloud deployment",
    tag = "deployments",
    description = "Changes the mode and the node range of the cluster, keeping its data plane. \
                   The node type cannot change, and the replicas running now cannot be taken \
                   away. The provider is changed first and the record second.",
    params(ClusterProfileRoute),
    request_body = ClusterProfileRequest,
    responses(
        (status = 200, description = "The profile now in force", body = ResizeClusterResponse),
        (status = 400, description = "Invalid request data", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such customer cloud deployment", body = ApiError),
        (status = 409, description = "The cluster is not ready to be resized", body = ApiError),
        (status = 422, description = "The profile or the resize is refused", body = ApiError),
        (status = 502, description = "The provider refused or could not serve the request", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn resize_cluster_handler(
    ClusterProfileRoute {
        organisation_id,
        deployment_id,
    }: ClusterProfileRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<ClusterProfileRequest>,
) -> Result<Response<ResizeClusterResponse>, ApiError> {
    let profile = state
        .service
        .resize_customer_cluster(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            request.into_spec()?,
        )
        .await?;

    Ok(Response::OK(ResizeClusterResponse { data: profile }))
}
