use autharie_auth::Identity;
use autharie_core::deployments::{
    Deployment, DeploymentName, commands::UpdateDeploymentCommand, ports::DeploymentService,
};
use axum::{
    Json,
    extract::{Extension, State, rejection::JsonRejection},
};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDeploymentRequest {
    pub name: Option<String>,
    /// Turns Herald's continuous shipping of this deployment's logs to the
    /// search index on or off (#294). The console does not drive this yet
    /// (V3, #296).
    pub log_shipping_enabled: Option<bool>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct UpdateDeploymentResponse {
    data: Deployment,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}")]
pub struct UpdateDeploymentRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

fn refused(rejection: JsonRejection) -> ApiError {
    match rejection {
        JsonRejection::JsonDataError(error) => ApiError::Unprocessable {
            reason: error.body_text(),
        },
        other => ApiError::BadRequest {
            reason: other.body_text(),
        },
    }
}

#[utoipa::path(
    patch,
    path = "/{organisation_id}/deployments/{deployment_id}",
    summary = "update deployment",
    tag = "deployments",
    description = "Update the settings of a deployment within the specified organisation: its name and its log shipping. Any other field is refused with a 422.",
    request_body = UpdateDeploymentRequest,
    params(UpdateDeploymentRoute),
    responses(
        (status = 200, description = "Deployment updated successfully", body = UpdateDeploymentResponse),
        (status = 400, description = "Invalid request data", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "The body carries a field that is not a deployment setting", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn update_deployment_handler(
    UpdateDeploymentRoute {
        organisation_id,
        deployment_id,
    }: UpdateDeploymentRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    request: Result<Json<UpdateDeploymentRequest>, JsonRejection>,
) -> Result<Response<UpdateDeploymentResponse>, ApiError> {
    let Json(request) = request.map_err(refused)?;
    let organisation_id = organisation_id.into();
    let deployment_id = deployment_id.into();

    let mut command = UpdateDeploymentCommand::new();
    if let Some(name) = request.name {
        command = command.with_name(DeploymentName(name))?;
    }
    if let Some(log_shipping_enabled) = request.log_shipping_enabled {
        command = command.with_log_shipping_enabled(log_shipping_enabled);
    }

    let deployment = state
        .service
        .update_deployment_for_organisation(identity, organisation_id, deployment_id, command)
        .await?;

    Ok(Response::OK(UpdateDeploymentResponse { data: deployment }))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use axum_extra::routing::RouterExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::test_helpers::{app_state, user_identity};

    async fn patch(body: &str) -> (StatusCode, Value) {
        let app = Router::new()
            .typed_patch(update_deployment_handler)
            .layer(Extension(user_identity(
                "9f3f7a4d-52a3-4a1a-9b3f-0c1b9b7d9a6f",
            )))
            .with_state(app_state());
        let request = Request::builder()
            .method("PATCH")
            .uri(format!(
                "/organisations/{}/deployments/{}",
                Uuid::new_v4(),
                Uuid::new_v4()
            ))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("a request");

        let response = app.oneshot(request).await.expect("a response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body");

        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn every_platform_field_is_refused_by_name() {
        for (field, value) in [
            ("status", r#""successful""#),
            ("namespace", r#""elsewhere""#),
            ("kind", r#""keycloak""#),
            ("version", r#""9.9.9""#),
            ("deployed_at", r#""2026-01-01T00:00:00Z""#),
            ("deleted_at", "null"),
        ] {
            let (status, body) = patch(&format!(r#"{{"{field}": {value}}}"#)).await;

            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {body}");
            assert_eq!(body["code"], "E_UNPROCESSABLE", "{field}: {body}");
            let message = body["message"].as_str().expect("a message");
            assert!(
                message.contains(&format!("`{field}`")),
                "{field}: {message}"
            );
        }
    }

    #[tokio::test]
    async fn a_platform_field_beside_a_setting_is_still_refused() {
        let (status, body) = patch(r#"{"name": "renamed", "status": "successful"}"#).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }

    #[tokio::test]
    async fn malformed_json_stays_a_bad_request() {
        let (status, body) = patch("{").await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}
