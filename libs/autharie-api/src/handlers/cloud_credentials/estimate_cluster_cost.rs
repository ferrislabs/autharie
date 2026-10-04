use autharie_auth::Identity;
use autharie_core::{
    cloud_credentials::CloudProviderService,
    dataplane::{
        cluster_profile::{ClusterMode, CostEstimate},
        credential::CloudCredentialId,
        value_objects::Region,
    },
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::profile::ClusterProfileRequest;
use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/cluster-profiles/estimate")]
pub struct EstimateClusterProfileRoute {
    pub organisation_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
pub struct EstimateClusterProfileRequest {
    pub credential_id: Uuid,
    pub region: String,
    pub mode: ClusterMode,
    pub control_plane_id: String,
    pub node_type: String,
    pub min_nodes: u8,
    pub max_nodes: u8,
    pub replication: u8,
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/cluster-profiles/estimate",
    summary = "estimate the monthly cost of a cluster profile",
    tag = "cloud-credentials",
    description = "Checks the profile against the provider's catalog and prices it: the nodes \
                   at the bottom and at the top of the range, each plus the control plane. \
                   Amounts are in minor units. Egress is not included.",
    request_body = EstimateClusterProfileRequest,
    params(EstimateClusterProfileRoute),
    responses(
        (status = 200, description = "The estimate", body = CostEstimate),
        (status = 400, description = "Invalid request data", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 404, description = "Credential not found", body = ApiError),
        (status = 422, description = "The profile is refused", body = ApiError),
        (status = 502, description = "The provider refused or could not serve the request", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn estimate_cluster_profile_handler(
    EstimateClusterProfileRoute { organisation_id }: EstimateClusterProfileRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<EstimateClusterProfileRequest>,
) -> Result<Response<CostEstimate>, ApiError> {
    let spec = ClusterProfileRequest {
        mode: request.mode,
        control_plane_id: request.control_plane_id,
        node_type: request.node_type,
        min_nodes: request.min_nodes,
        max_nodes: request.max_nodes,
        replication: request.replication,
    }
    .into_spec()?;

    let estimate = state
        .service
        .estimate_cluster_cost(
            identity,
            organisation_id.into(),
            CloudCredentialId(request.credential_id),
            Region::new(request.region),
            spec,
        )
        .await?;

    Ok(Response::OK(estimate))
}
