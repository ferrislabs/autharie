use autharie_auth::Identity;
use autharie_core::{
    deployments::{
        DeploymentId,
        reachability_history::{AvailabilityHistory, MAX_HISTORY_DAYS},
    },
    organisation::OrganisationId,
};
use axum::{
    Extension,
    extract::{Query, State},
};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/availability")]
pub struct AvailabilityRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AvailabilityQuery {
    /// How many UTC calendar days to return, ending today. 1 to 90, 90 when
    /// omitted.
    pub days: Option<u32>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct AvailabilityResponse {
    pub data: AvailabilityHistory,
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/availability",
    summary = "read a deployment's day-by-day availability",
    tag = "deployments",
    description = "One entry per UTC calendar day ending today, oldest first, from the \
                   deployment's recorded health checks. A day without any check has zero \
                   checks and a null uptime. Uptime is a fraction between 0 and 1 and is not \
                   rounded. Requires VIEW_INSTANCES.",
    params(AvailabilityRoute, AvailabilityQuery),
    responses(
        (status = 200, description = "The deployment's availability history", body = AvailabilityResponse),
        (status = 400, description = "days is not between 1 and 90", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's deployments", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_deployment_availability_handler(
    AvailabilityRoute {
        organisation_id,
        deployment_id,
    }: AvailabilityRoute,
    Query(query): Query<AvailabilityQuery>,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<AvailabilityResponse>, ApiError> {
    let days = query.days.unwrap_or(MAX_HISTORY_DAYS);
    if !(1..=MAX_HISTORY_DAYS).contains(&days) {
        return Err(ApiError::BadRequest {
            reason: format!("days must be between 1 and {MAX_HISTORY_DAYS}, got {days}"),
        });
    }

    let history = state
        .service
        .get_deployment_availability(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            days,
        )
        .await?;

    Ok(Response::OK(AvailabilityResponse { data: history }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{app_state, user_identity};

    #[tokio::test]
    async fn days_outside_one_to_ninety_are_rejected() {
        for days in [0, 91] {
            let result = get_deployment_availability_handler(
                AvailabilityRoute {
                    organisation_id: Uuid::new_v4(),
                    deployment_id: Uuid::new_v4(),
                },
                Query(AvailabilityQuery { days: Some(days) }),
                State(app_state()),
                Extension(user_identity("user-1")),
            )
            .await;

            assert!(matches!(result, Err(ApiError::BadRequest { .. })));
        }
    }
}
