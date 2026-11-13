use autharie_auth::Identity;
use autharie_core::{
    offers::Offer,
    organisation::OrganisationId,
    quotas::{OrganisationQuota, Quota, QuotaKind, ports::OrganisationQuotaService},
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/quotas")]
pub struct TenantQuotasRoute {
    pub organisation_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/quotas/{kind}")]
pub struct TenantQuotaRoute {
    pub organisation_id: Uuid,
    pub kind: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct OfferDefaultView {
    pub offer: Offer,
    pub quota: Quota,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantQuotaView {
    pub kind: QuotaKind,
    pub description: String,
    pub enforced: bool,
    pub defaults: Vec<OfferDefaultView>,
    #[serde(rename = "override")]
    pub organisation_override: Option<Quota>,
}

impl From<OrganisationQuota> for TenantQuotaView {
    fn from(view: OrganisationQuota) -> Self {
        Self {
            kind: view.kind,
            description: view.description.to_string(),
            enforced: view.enforced,
            defaults: view
                .defaults
                .into_iter()
                .map(|default| OfferDefaultView {
                    offer: default.offer,
                    quota: default.quota,
                })
                .collect(),
            organisation_override: view.organisation_override,
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantQuotasResponse {
    pub data: Vec<TenantQuotaView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantQuotaResponse {
    pub data: TenantQuotaView,
}

#[derive(Deserialize, ToSchema)]
pub struct SetTenantQuotaRequest {
    /// A limit, or `{"type": "unlimited"}`. `null` clears the override.
    #[serde(deserialize_with = "Option::deserialize")]
    pub quota: Option<Quota>,
}

#[utoipa::path(
    get,
    path = "/organisations/{organisation_id}/quotas",
    summary = "list the quotas of an organisation",
    tag = "platform",
    description = "Each quota kind with the default of every offer and the organisation's own \
                   override when it has one. The override applies to every deployment of the \
                   organisation, whatever its plan.",
    params(TenantQuotasRoute),
    responses(
        (status = 200, description = "Every quota kind as this organisation sees it", body = TenantQuotasResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_tenant_quotas_handler(
    TenantQuotasRoute { organisation_id }: TenantQuotasRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<TenantQuotasResponse>, ApiError> {
    let quotas = state
        .service
        .list_organisation_quotas(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(TenantQuotasResponse {
        data: quotas.into_iter().map(TenantQuotaView::from).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/organisations/{organisation_id}/quotas/{kind}",
    summary = "set or clear a quota override for an organisation",
    tag = "platform",
    description = "A limit or `unlimited` replaces the offer default for every deployment of \
                   this organisation; `null` clears the override. Lowering a quota below what a \
                   deployment holds deletes nothing and refuses only the next create.",
    params(TenantQuotaRoute),
    request_body = SetTenantQuotaRequest,
    responses(
        (status = 200, description = "The quota as this organisation now sees it", body = TenantQuotaResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the act_on_tenant right", body = ApiError),
        (status = 404, description = "No such organisation or quota", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_tenant_quota_handler(
    TenantQuotaRoute {
        organisation_id,
        kind,
    }: TenantQuotaRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<SetTenantQuotaRequest>,
) -> Result<Response<TenantQuotaResponse>, ApiError> {
    let kind = kind.parse::<QuotaKind>()?;

    let view = state
        .service
        .set_organisation_quota(
            identity,
            OrganisationId(organisation_id),
            kind,
            request.quota,
        )
        .await?;

    Ok(Response::OK(TenantQuotaResponse { data: view.into() }))
}
