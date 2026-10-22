use autharie_auth::Identity;
use autharie_core::deployments::power::DeploymentPowerService;
use axum::extract::{Extension, State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    errors::ApiError, handlers::deployments::deployment_view::DeploymentView, response::Response,
    state::AppState,
};

#[derive(Serialize, ToSchema, PartialEq)]
pub struct DeploymentPowerResponse {
    data: DeploymentView,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/stop")]
pub struct StopDeploymentRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/start")]
pub struct StartDeploymentRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/stop",
    summary = "stop a deployment",
    tag = "deployments",
    description = "Powers a deployment down, its data kept. It stays billed while stopped. Needs \
                   the right to manage deployments. A deployment that has not started, is being \
                   upgraded, or is on its way out has nothing to stop. One order at a time: it is \
                   refused while an earlier stop or start has not reached its data plane, and \
                   for as long as the deployment is in progress, whether it is being created or \
                   started.",
    params(StopDeploymentRoute),
    responses(
        (status = 200, description = "The deployment, stopped", body = DeploymentPowerResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage deployments here", body = ApiError),
        (status = 404, description = "Deployment not found", body = ApiError),
        (status = 409, description = "The deployment cannot be stopped in its state, or an earlier order is unfinished", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn stop_deployment_handler(
    StopDeploymentRoute {
        organisation_id,
        deployment_id,
    }: StopDeploymentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<DeploymentPowerResponse>, ApiError> {
    let deployment = state
        .service
        .stop_deployment(identity, organisation_id.into(), deployment_id.into())
        .await?;

    Ok(Response::OK(DeploymentPowerResponse {
        data: DeploymentView::new(deployment, state.args.ovh.domain().as_deref()),
    }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/start",
    summary = "start a stopped deployment",
    tag = "deployments",
    description = "Powers a stopped deployment back up. It comes back in progress, and is running \
                   once its data plane says so. Needs the right to manage deployments. Refused \
                   for as long as billing has the organisation suspended, since that is when \
                   billing stops deployments and it starts them again itself once the invoice is \
                   paid. One order at a time: it is refused while an earlier stop or start has \
                   not reached its data plane.",
    params(StartDeploymentRoute),
    responses(
        (status = 200, description = "The deployment, starting", body = DeploymentPowerResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage deployments here", body = ApiError),
        (status = 404, description = "Deployment not found", body = ApiError),
        (status = 409, description = "The deployment is not stopped, billing has the organisation suspended, or an earlier order is unfinished", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn start_deployment_handler(
    StartDeploymentRoute {
        organisation_id,
        deployment_id,
    }: StartDeploymentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<DeploymentPowerResponse>, ApiError> {
    let deployment = state
        .service
        .start_deployment(identity, organisation_id.into(), deployment_id.into())
        .await?;

    Ok(Response::OK(DeploymentPowerResponse {
        data: DeploymentView::new(deployment, state.args.ovh.domain().as_deref()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{app_state, user_identity};

    #[tokio::test]
    async fn stop_propagates_the_service_error_instead_of_flattening_it() {
        let result = stop_deployment_handler(
            StopDeploymentRoute {
                organisation_id: Uuid::new_v4(),
                deployment_id: Uuid::new_v4(),
            },
            State(app_state()),
            Extension(user_identity("9f3f7a4d-52a3-4a1a-9b3f-0c1b9b7d9a6f")),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn start_propagates_the_service_error_instead_of_flattening_it() {
        let result = start_deployment_handler(
            StartDeploymentRoute {
                organisation_id: Uuid::new_v4(),
                deployment_id: Uuid::new_v4(),
            },
            State(app_state()),
            Extension(user_identity("9f3f7a4d-52a3-4a1a-9b3f-0c1b9b7d9a6f")),
        )
        .await;

        assert!(result.is_err());
    }
}
