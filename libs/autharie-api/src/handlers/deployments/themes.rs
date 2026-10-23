use autharie_auth::Identity;
use autharie_core::{
    deployments::DeploymentId,
    organisation::OrganisationId,
    themes::{
        ThemeSync, ThemeTokens, ThemeView,
        commands::{NewTheme, ThemeChange},
        parse_realm,
        ports::ThemeService,
        value_objects::{ThemeId, ThemeRealm},
    },
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes"
)]
pub struct ThemesRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub realm: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/{theme_id}"
)]
pub struct ThemeRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub realm: String,
    pub theme_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/{theme_id}/activate"
)]
pub struct ThemeActivateRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub realm: String,
    pub theme_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path(
    "/organisations/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/deactivate"
)]
pub struct ThemesDeactivateRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub realm: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateThemeBody {
    #[schema(min_length = 1, max_length = 80, example = "Autumn")]
    pub name: String,
    #[schema(value_type = ThemeTokens)]
    pub tokens: serde_json::Value,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateThemeBody {
    #[serde(default)]
    #[schema(min_length = 1, max_length = 80)]
    pub name: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<ThemeTokens>)]
    pub tokens: Option<serde_json::Value>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ThemesResponse {
    pub data: Vec<ThemeView>,
    #[schema(required = true)]
    pub state: Option<ThemeSync>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ThemeResponse {
    pub data: ThemeView,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct ThemesDeactivated {}

fn realm_of(realm: &str) -> Result<ThemeRealm, ApiError> {
    parse_realm(realm).map_err(|error| ApiError::BadRequest {
        reason: error.to_string(),
    })
}

fn bad(error: autharie_core::themes::ThemeError) -> ApiError {
    ApiError::BadRequest {
        reason: error.to_string(),
    }
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes",
    summary = "list a realm's themes",
    tag = "branding",
    description = "The themes of the realm, oldest first, and `state`: what became of the tokens last sent to FerrisKey (`Pending`, `Applied` or `Failed`, with the `revision` of that send), or null when none was ever sent. Allowed whatever the plan.",
    params(ThemesRoute),
    responses(
        (status = 200, description = "The realm's themes", body = ThemesResponse),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_themes_handler(
    ThemesRoute {
        organisation_id,
        deployment_id,
        realm,
    }: ThemesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<ThemesResponse>, ApiError> {
    let listing = state
        .service
        .list_themes(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm_of(&realm)?,
        )
        .await?;

    Ok(Response::OK(ThemesResponse {
        data: listing.themes.into_iter().map(ThemeView::from).collect(),
        state: listing.state,
    }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes",
    summary = "create a theme",
    tag = "branding",
    description = "Stores a new, inactive theme. Nothing is sent to the data plane until it is activated. A realm holds at most 10 themes and a name is unique in a realm whatever its case. Needs the branding feature. FerrisKey deployments only.",
    params(ThemesRoute),
    request_body = CreateThemeBody,
    responses(
        (status = 201, description = "The theme was created", body = ThemeResponse),
        (status = 400, description = "The name, a token (named in the message) or the realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment, or its plan does not open branding", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The realm already holds 10 themes, the name is taken, or the deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_theme_handler(
    ThemesRoute {
        organisation_id,
        deployment_id,
        realm,
    }: ThemesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<CreateThemeBody>,
) -> Result<Response<ThemeResponse>, ApiError> {
    let realm = realm_of(&realm)?;
    let theme = NewTheme::parse(&body.name, &body.tokens).map_err(bad)?;

    let created = state
        .service
        .create_theme(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm,
            theme,
        )
        .await?;

    Ok(Response::Created(ThemeResponse {
        data: ThemeView::from(created),
    }))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/{theme_id}",
    summary = "change a theme",
    tag = "branding",
    description = "Renames the theme, replaces its tokens, or both. The data plane hears of it only when the theme is the active one and its tokens change. Needs the branding feature.",
    params(ThemeRoute),
    request_body = UpdateThemeBody,
    responses(
        (status = 200, description = "The theme as it is now", body = ThemeResponse),
        (status = 400, description = "Nothing to change, or the name, a token (named in the message) or the realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment, or its plan does not open branding", body = ApiError),
        (status = 404, description = "No such deployment or theme", body = ApiError),
        (status = 409, description = "The name is taken, or the deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_theme_handler(
    ThemeRoute {
        organisation_id,
        deployment_id,
        realm,
        theme_id,
    }: ThemeRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<UpdateThemeBody>,
) -> Result<Response<ThemeResponse>, ApiError> {
    let realm = realm_of(&realm)?;
    let change = ThemeChange::parse(body.name.as_deref(), body.tokens.as_ref()).map_err(bad)?;

    let updated = state
        .service
        .update_theme(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm,
            ThemeId(theme_id),
            change,
        )
        .await?;

    Ok(Response::OK(ThemeResponse {
        data: ThemeView::from(updated),
    }))
}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/{theme_id}",
    summary = "delete a theme",
    tag = "branding",
    description = "Forgets a theme that is not active. Nothing is sent to the data plane. Needs the branding feature.",
    params(ThemeRoute),
    responses(
        (status = 204, description = "The theme was deleted"),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment, or its plan does not open branding", body = ApiError),
        (status = 404, description = "No such deployment or theme", body = ApiError),
        (status = 409, description = "The theme is the active one, or the deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_theme_handler(
    ThemeRoute {
        organisation_id,
        deployment_id,
        realm,
        theme_id,
    }: ThemeRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<()>, ApiError> {
    state
        .service
        .delete_theme(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm_of(&realm)?,
            ThemeId(theme_id),
        )
        .await?;

    Ok(Response::NoContent)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/{theme_id}/activate",
    summary = "activate a theme",
    tag = "branding",
    description = "Makes the theme the realm's only active one and asks the data plane to apply its tokens as FerrisKey's active theme. `state` of the list is `Pending` until the data plane reports. Needs the branding feature.",
    params(ThemeActivateRoute),
    responses(
        (status = 202, description = "Active, and being applied", body = ThemeResponse),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment, or its plan does not open branding", body = ApiError),
        (status = 404, description = "No such deployment or theme", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn activate_theme_handler(
    ThemeActivateRoute {
        organisation_id,
        deployment_id,
        realm,
        theme_id,
    }: ThemeActivateRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<ThemeResponse>, ApiError> {
    let activated = state
        .service
        .activate_theme(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm_of(&realm)?,
            ThemeId(theme_id),
        )
        .await?;

    Ok(Response::Accepted(ThemeResponse {
        data: ThemeView::from(activated),
    }))
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes/deactivate",
    summary = "use FerrisKey's default look",
    tag = "branding",
    description = "Leaves the realm with no active theme and asks the data plane to apply empty tokens, which is FerrisKey's own default. Needs the branding feature.",
    params(ThemesDeactivateRoute),
    responses(
        (status = 202, description = "No theme is active, and the default is being applied", body = ThemesDeactivated),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment, or its plan does not open branding", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn deactivate_themes_handler(
    ThemesDeactivateRoute {
        organisation_id,
        deployment_id,
        realm,
    }: ThemesDeactivateRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<ThemesDeactivated>, ApiError> {
    state
        .service
        .deactivate_themes(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm_of(&realm)?,
        )
        .await?;

    Ok(Response::Accepted(ThemesDeactivated {}))
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use serde_json::json;

    use super::*;

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    fn create(value: serde_json::Value) -> Result<NewTheme, ApiError> {
        let body: CreateThemeBody = serde_json::from_value(value).expect("a body");
        NewTheme::parse(&body.name, &body.tokens).map_err(bad)
    }

    #[test]
    fn a_valid_body_is_read() {
        let theme = create(json!({
            "name": "Autumn",
            "tokens": { "colors": { "primaryButton": "#112233" } }
        }))
        .expect("accepted");

        assert_eq!(theme.name.as_str(), "Autumn");
    }

    #[test]
    fn a_bad_token_is_a_400_that_names_the_field() {
        for (tokens, field) in [
            (
                json!({ "colors": { "primaryButton": "red" } }),
                "colors.primaryButton",
            ),
            (
                json!({ "borders": { "buttonRadius": 65 } }),
                "borders.buttonRadius",
            ),
            (json!({ "fonts": { "baseSize": 9 } }), "fonts.baseSize"),
            (json!({ "colours": {} }), "colours"),
        ] {
            let error = create(json!({ "name": "Autumn", "tokens": tokens })).expect_err("refused");

            let ApiError::BadRequest { reason } = &error else {
                panic!("expected a 400");
            };
            assert!(reason.starts_with(&format!("{field}:")), "{reason}");
            assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn an_unknown_field_of_the_body_is_not_read() {
        let extra = json!({ "name": "Autumn", "tokens": {}, "active": true });

        assert!(serde_json::from_value::<CreateThemeBody>(extra).is_err());
        assert!(serde_json::from_value::<UpdateThemeBody>(json!({ "active": true })).is_err());
    }

    #[test]
    fn an_empty_update_is_a_400() {
        let body: UpdateThemeBody = serde_json::from_value(json!({})).expect("a body");

        let error = ThemeChange::parse(body.name.as_deref(), body.tokens.as_ref())
            .map_err(bad)
            .expect_err("refused");

        assert_eq!(status_of(error), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_bad_realm_is_a_400() {
        assert_eq!(
            status_of(realm_of("bad realm").expect_err("refused")),
            StatusCode::BAD_REQUEST
        );
        assert!(realm_of("master").is_ok());
    }

    #[test]
    fn the_list_shows_a_null_state_rather_than_leaving_it_out() {
        let value = serde_json::to_value(ThemesResponse {
            data: vec![],
            state: None,
        })
        .unwrap();

        assert_eq!(value, json!({ "data": [], "state": null }));
    }

    #[test]
    fn deactivating_answers_an_empty_object() {
        assert_eq!(
            serde_json::to_value(ThemesDeactivated {}).unwrap(),
            json!({})
        );
    }
}
