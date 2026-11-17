use autharie_auth::Identity;
use autharie_core::{
    organisation::OrganisationId,
    quotas::{
        OrganisationQuotaKind, OrganisationScopeQuota, Quota, ports::OrganisationScopeQuotaService,
    },
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/organisation-quotas")]
pub struct TenantOrganisationQuotasRoute {
    pub organisation_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/organisation-quotas/{kind}")]
pub struct TenantOrganisationQuotaRoute {
    pub organisation_id: Uuid,
    pub kind: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantOrganisationQuotaView {
    pub kind: OrganisationQuotaKind,
    pub description: String,
    pub plan_default: Quota,
    #[serde(rename = "override")]
    pub organisation_override: Option<Quota>,
    pub effective: Quota,
    pub held: usize,
}

impl From<OrganisationScopeQuota> for TenantOrganisationQuotaView {
    fn from(view: OrganisationScopeQuota) -> Self {
        Self {
            kind: view.kind,
            description: view.description.to_string(),
            plan_default: view.plan_default,
            organisation_override: view.organisation_override,
            effective: view.effective,
            held: view.held,
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantOrganisationQuotasResponse {
    pub data: Vec<TenantOrganisationQuotaView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantOrganisationQuotaResponse {
    pub data: TenantOrganisationQuotaView,
}

#[derive(Deserialize, ToSchema)]
pub struct SetTenantOrganisationQuotaRequest {
    /// A limit, or `{"type": "unlimited"}`. `null` clears the override.
    #[serde(deserialize_with = "Option::deserialize")]
    pub quota: Option<Quota>,
}

#[utoipa::path(
    get,
    path = "/organisations/{organisation_id}/organisation-quotas",
    summary = "list the organisation-wide quotas of an organisation",
    tag = "platform",
    description = "The deployments and members quotas: the default of the organisation's plan, \
                   its own override when it has one, the effective limit and what it holds now.",
    params(TenantOrganisationQuotasRoute),
    responses(
        (status = 200, description = "Every organisation-wide quota", body = TenantOrganisationQuotasResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_tenant_organisation_quotas_handler(
    TenantOrganisationQuotasRoute { organisation_id }: TenantOrganisationQuotasRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<TenantOrganisationQuotasResponse>, ApiError> {
    let quotas = state
        .service
        .list_organisation_scope_quotas(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(TenantOrganisationQuotasResponse {
        data: quotas
            .into_iter()
            .map(TenantOrganisationQuotaView::from)
            .collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/organisations/{organisation_id}/organisation-quotas/{kind}",
    summary = "set or clear an organisation-wide quota override",
    tag = "platform",
    description = "A limit or `unlimited` replaces the plan default; `null` clears the override. \
                   Lowering a quota below what the organisation holds deletes nothing and refuses \
                   only the next create.",
    params(TenantOrganisationQuotaRoute),
    request_body = SetTenantOrganisationQuotaRequest,
    responses(
        (status = 200, description = "The quota as the organisation now sees it", body = TenantOrganisationQuotaResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the act_on_tenant right", body = ApiError),
        (status = 404, description = "No such organisation or quota", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_tenant_organisation_quota_handler(
    TenantOrganisationQuotaRoute {
        organisation_id,
        kind,
    }: TenantOrganisationQuotaRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<SetTenantOrganisationQuotaRequest>,
) -> Result<Response<TenantOrganisationQuotaResponse>, ApiError> {
    let kind = kind.parse::<OrganisationQuotaKind>()?;

    let view = state
        .service
        .set_organisation_scope_quota(
            identity,
            OrganisationId(organisation_id),
            kind,
            request.quota,
        )
        .await?;

    Ok(Response::OK(TenantOrganisationQuotaResponse {
        data: view.into(),
    }))
}
