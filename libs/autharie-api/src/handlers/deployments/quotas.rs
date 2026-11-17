use autharie_auth::Identity;
use autharie_core::{
    deployments::DeploymentId,
    organisation::OrganisationId,
    quotas::{EffectiveQuota, Quota, QuotaKind, ports::DeploymentQuotaService},
};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/quotas")]
pub struct DeploymentQuotasRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct DeploymentQuotaView {
    pub kind: QuotaKind,
    pub quota: Quota,
    /// Whether platform staff replaced the offer's default for this organisation.
    pub overridden: bool,
    /// Whether going over refuses anything.
    pub enforced: bool,
}

impl From<EffectiveQuota> for DeploymentQuotaView {
    fn from(view: EffectiveQuota) -> Self {
        Self {
            kind: view.kind,
            quota: view.quota,
            overridden: view.overridden,
            enforced: view.kind.is_enforced(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct DeploymentQuotasResponse {
    pub data: Vec<DeploymentQuotaView>,
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/quotas",
    summary = "list what a deployment may hold",
    tag = "deployments",
    description = "The quota of each kind that applies to this deployment: its offer's default, \
                   or the override set for its organisation. A deployment created before offers \
                   existed has no default and is limited only by an override.",
    params(DeploymentQuotasRoute),
    responses(
        (status = 200, description = "The deployment's quotas", body = DeploymentQuotasResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's deployments", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_deployment_quotas_handler(
    DeploymentQuotasRoute {
        organisation_id,
        deployment_id,
    }: DeploymentQuotasRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<DeploymentQuotasResponse>, ApiError> {
    let quotas = state
        .service
        .deployment_quotas(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(DeploymentQuotasResponse {
        data: quotas.into_iter().map(DeploymentQuotaView::from).collect(),
    }))
}
