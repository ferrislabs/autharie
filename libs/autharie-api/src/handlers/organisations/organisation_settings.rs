use autharie_auth::Identity;
use autharie_core::organisation::{
    Organisation, OrganisationId, commands::UpdateOrganisationCommand, ports::OrganisationSettings,
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

#[derive(Serialize, ToSchema, PartialEq)]
pub struct OrganisationResponse {
    data: Organisation,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateOrganisationRequest {
    pub mcp_enabled: Option<bool>,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}")]
pub struct OrganisationRoute {
    pub organisation_id: Uuid,
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
    get,
    path = "/{organisation_id}",
    summary = "get organisation",
    tag = "organisation",
    description = "Read an organisation the caller is a member of, with its settings.",
    params(OrganisationRoute),
    responses(
        (status = 200, description = "The organisation", body = OrganisationResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not see this organisation", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_organisation_handler(
    OrganisationRoute { organisation_id }: OrganisationRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<OrganisationResponse>, ApiError> {
    let organisation = state
        .service
        .get_organisation(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(OrganisationResponse { data: organisation }))
}

#[utoipa::path(
    patch,
    path = "/{organisation_id}",
    summary = "update organisation",
    tag = "organisation",
    description = "Change an organisation's settings: whether AI agents may act on it through MCP. Needs the right to manage the organisation, and is refused to a delegated (AI agent) token. Any other field is refused with a 422.",
    request_body = UpdateOrganisationRequest,
    params(OrganisationRoute),
    responses(
        (status = 200, description = "Organisation updated", body = OrganisationResponse),
        (status = 400, description = "Nothing to change", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage this organisation", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 422, description = "The body carries a field that is not a setting", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn update_organisation_handler(
    OrganisationRoute { organisation_id }: OrganisationRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    request: Result<Json<UpdateOrganisationRequest>, JsonRejection>,
) -> Result<Response<OrganisationResponse>, ApiError> {
    let Json(request) = request.map_err(refused)?;

    let mut command = UpdateOrganisationCommand::new();
    if let Some(enabled) = request.mcp_enabled {
        command = command.with_mcp_enabled(enabled);
    }
    if command.is_empty() {
        return Err(ApiError::BadRequest {
            reason: "nothing to change".to_string(),
        });
    }

    let organisation = state
        .service
        .change_organisation(identity, OrganisationId(organisation_id), command)
        .await?;

    Ok(Response::OK(OrganisationResponse { data: organisation }))
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
            .typed_patch(update_organisation_handler)
            .layer(Extension(user_identity(
                "9f3f7a4d-52a3-4a1a-9b3f-0c1b9b7d9a6f",
            )))
            .with_state(app_state());
        let request = Request::builder()
            .method("PATCH")
            .uri(format!("/organisations/{}", Uuid::new_v4()))
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
    async fn an_empty_patch_changes_nothing_and_says_so() {
        let (status, _) = patch("{}").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_field_that_is_not_a_setting_is_refused() {
        let (status, _) = patch(r#"{"plan":"enterprise"}"#).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn the_switch_reaches_the_service() {
        let (_, body) = patch(r#"{"mcp_enabled":true}"#).await;

        assert_eq!(body["code"], "E_UNKNOWN");
    }
}
