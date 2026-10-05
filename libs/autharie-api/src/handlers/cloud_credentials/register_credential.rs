use autharie_auth::Identity;
use autharie_core::{
    cloud_credentials::CloudProviderService,
    dataplane::{
        cloud_provider::Provider,
        credential::{CloudCredential, SecretString},
    },
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

const MAX_LABEL_CHARS: usize = 100;

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/cloud-credentials")]
pub struct CloudCredentialsRoute {
    pub organisation_id: Uuid,
}

#[derive(Deserialize, ToSchema)]
pub struct RegisterCloudCredentialRequest {
    pub provider: Provider,
    pub label: String,
    #[schema(write_only)]
    pub secret: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct CloudCredentialResponse {
    pub id: Uuid,
    pub provider: Provider,
    pub label: String,
    pub scope_checked_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl From<CloudCredential> for CloudCredentialResponse {
    fn from(credential: CloudCredential) -> Self {
        Self {
            id: credential.id.0,
            provider: credential.provider,
            label: credential.label,
            scope_checked_at: credential.scope_check.checked_at,
            created_at: credential.created_at,
        }
    }
}

fn checked_label(label: String) -> Result<String, ApiError> {
    let label = label.trim().to_string();

    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(ApiError::BadRequest {
            reason: format!("a label is 1 to {MAX_LABEL_CHARS} characters"),
        });
    }

    Ok(label)
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/cloud-credentials",
    summary = "register a cloud credential",
    tag = "cloud-credentials",
    description = "Registers a provider key for the organisation. The key is checked for the \
                   permissions it needs and no more, then stored encrypted. It is never \
                   returned.",
    request_body = RegisterCloudCredentialRequest,
    params(CloudCredentialsRoute),
    responses(
        (status = 201, description = "Credential registered", body = CloudCredentialResponse),
        (status = 400, description = "Invalid request data", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 422, description = "The key is invalid or has missing or excess permissions", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn register_cloud_credential_handler(
    CloudCredentialsRoute { organisation_id }: CloudCredentialsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<RegisterCloudCredentialRequest>,
) -> Result<Response<CloudCredentialResponse>, ApiError> {
    let label = checked_label(request.label)?;
    let secret = SecretString::new(request.secret);

    let credential = state
        .service
        .register_cloud_credential(
            identity,
            organisation_id.into(),
            request.provider,
            label,
            secret,
        )
        .await?;

    Ok(Response::Created(credential.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_trimmed_and_bounded() {
        assert_eq!(checked_label("  prod ".to_string()).expect("ok"), "prod");
        assert!(checked_label("   ".to_string()).is_err());
        assert!(checked_label("x".repeat(101)).is_err());
    }

    #[test]
    fn the_response_has_no_secret_and_no_organisation() {
        let json = serde_json::to_value(CloudCredentialResponse {
            id: Uuid::nil(),
            provider: Provider::Scaleway,
            label: "prod".to_string(),
            scope_checked_at: Utc::now(),
            created_at: Utc::now(),
        })
        .expect("serialised");

        let mut keys: Vec<_> = json
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();

        assert_eq!(
            keys,
            ["created_at", "id", "label", "provider", "scope_checked_at"]
        );
    }
}
