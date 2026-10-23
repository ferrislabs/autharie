use autharie_auth::Identity;
use autharie_core::{
    deployments::DeploymentId,
    organisation::OrganisationId,
    smtp::{
        SmtpEncryption, SmtpView,
        commands::{RawSmtpInput, SmtpInput},
        ports::SmtpService,
        value_objects::{SmtpRealm, parse_realm},
    },
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/branding/{realm}/smtp")]
pub struct SmtpRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
    pub realm: String,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/branding/realms")]
pub struct BrandingRealmsRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SmtpRequestBody {
    pub host: String,
    #[schema(minimum = 1, maximum = 65535, example = 587)]
    pub port: i64,
    pub encryption: SmtpEncryption,
    pub username: String,
    pub password: Option<String>,
    pub from_email: String,
    pub from_name: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct SmtpResponse {
    pub data: Option<SmtpView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct SmtpSavedResponse {
    pub data: SmtpView,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct SmtpRemoved {}

fn realm_of(realm: &str) -> Result<SmtpRealm, ApiError> {
    parse_realm(realm).map_err(|error| ApiError::BadRequest {
        reason: error.to_string(),
    })
}

fn input_from(realm: &str, body: SmtpRequestBody) -> Result<SmtpInput, ApiError> {
    SmtpInput::parse(
        realm,
        RawSmtpInput {
            host: body.host,
            port: body.port,
            encryption: body.encryption.as_str().to_string(),
            username: body.username,
            password: body.password,
            from_email: body.from_email,
            from_name: body.from_name,
        },
    )
    .map_err(|error| ApiError::BadRequest {
        reason: error.to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/smtp",
    summary = "read a realm's email provider",
    tag = "branding",
    description = "`data` is null when the realm has no email provider. The password is never returned: `password_set` says one is stored. The state is `Pending` until the data plane reports the provider applied.",
    params(SmtpRoute),
    responses(
        (status = 200, description = "The realm's email provider, or null", body = SmtpResponse),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_smtp_handler(
    SmtpRoute {
        organisation_id,
        deployment_id,
        realm,
    }: SmtpRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<SmtpResponse>, ApiError> {
    let data = state
        .service
        .get_smtp(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm_of(&realm)?,
        )
        .await?;

    Ok(Response::OK(SmtpResponse { data }))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/smtp",
    summary = "set a realm's email provider",
    tag = "branding",
    description = "Stores the provider as `Pending` and asks the data plane to apply it. The password is required the first time and optional after: left out, the stored one is kept. It is sealed on arrival and never returned. FerrisKey deployments only.",
    params(SmtpRoute),
    request_body = SmtpRequestBody,
    responses(
        (status = 202, description = "Stored, and being applied", body = SmtpSavedResponse),
        (status = 400, description = "The host, port, address, a length or the realm is not valid, or the first save carries no password", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_smtp_handler(
    SmtpRoute {
        organisation_id,
        deployment_id,
        realm,
    }: SmtpRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(body): Json<SmtpRequestBody>,
) -> Result<Response<SmtpSavedResponse>, ApiError> {
    let input = input_from(&realm, body)?;

    let data = state
        .service
        .save_smtp(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            input,
        )
        .await?;

    Ok(Response::Accepted(SmtpSavedResponse { data }))
}

#[utoipa::path(
    delete,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/{realm}/smtp",
    summary = "remove a realm's email provider",
    tag = "branding",
    description = "Forgets the provider and asks the data plane to remove it from FerrisKey.",
    params(SmtpRoute),
    responses(
        (status = 202, description = "Removed, and being applied", body = SmtpRemoved),
        (status = 400, description = "The realm is not valid", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not change this deployment", body = ApiError),
        (status = 404, description = "No such deployment, or the realm has no email provider", body = ApiError),
        (status = 409, description = "The deployment is not a FerrisKey one", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_smtp_handler(
    SmtpRoute {
        organisation_id,
        deployment_id,
        realm,
    }: SmtpRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<SmtpRemoved>, ApiError> {
    state
        .service
        .remove_smtp(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            realm_of(&realm)?,
        )
        .await?;

    Ok(Response::Accepted(SmtpRemoved {}))
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct BrandingRealmsResponse {
    pub data: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/branding/realms",
    summary = "list the realms the branding can be set for",
    tag = "branding",
    description = "The realms the data plane last reported, or `master` alone until it reports any. The same list as the webhooks' realms.",
    params(BrandingRealmsRoute),
    responses(
        (status = 200, description = "The realm names", body = BrandingRealmsResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this deployment", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_branding_realms_handler(
    BrandingRealmsRoute {
        organisation_id,
        deployment_id,
    }: BrandingRealmsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<BrandingRealmsResponse>, ApiError> {
    let data = state
        .service
        .branding_realms(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(BrandingRealmsResponse { data }))
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};
    use serde_json::json;

    use super::*;

    fn body(value: serde_json::Value) -> SmtpRequestBody {
        serde_json::from_value(value).expect("a body")
    }

    fn valid() -> serde_json::Value {
        json!({
            "host": "smtp.example.com",
            "port": 587,
            "encryption": "starttls",
            "username": "apikey",
            "password": "secret",
            "from_email": "no-reply@example.com",
            "from_name": "Acme"
        })
    }

    fn status_of(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    #[test]
    fn a_valid_body_is_read() {
        let input = input_from("master", body(valid())).expect("accepted");

        assert_eq!(input.realm.as_str(), "master");
        assert_eq!(input.settings.port.value(), 587);
        assert_eq!(input.password.expect("a password").expose(), "secret");
    }

    #[test]
    fn a_body_without_a_password_is_read() {
        let mut keep = valid();
        keep.as_object_mut().unwrap().remove("password");

        assert!(
            input_from("master", body(keep))
                .expect("accepted")
                .password
                .is_none()
        );
    }

    #[test]
    fn a_bad_host_port_or_address_is_a_400() {
        for (field, value) in [
            ("host", json!("smtp://mail.example.com")),
            ("port", json!(0)),
            ("port", json!(70000)),
            ("from_email", json!("nobody")),
            ("username", json!("")),
        ] {
            let mut bad = valid();
            bad[field] = value;

            assert_eq!(
                status_of(input_from("master", body(bad)).expect_err("refused")),
                StatusCode::BAD_REQUEST,
                "{field}"
            );
        }
    }

    #[test]
    fn a_bad_realm_is_a_400() {
        assert_eq!(
            status_of(input_from("bad realm", body(valid())).expect_err("refused")),
            StatusCode::BAD_REQUEST
        );
        assert!(realm_of("master").is_ok());
        assert!(realm_of("a/b").is_err());
    }

    #[test]
    fn an_unknown_field_or_encryption_is_not_read() {
        let mut extra = valid();
        extra["secure"] = json!(true);
        assert!(serde_json::from_value::<SmtpRequestBody>(extra).is_err());

        let mut ssl = valid();
        ssl["encryption"] = json!("ssl");
        assert!(serde_json::from_value::<SmtpRequestBody>(ssl).is_err());
    }
}
