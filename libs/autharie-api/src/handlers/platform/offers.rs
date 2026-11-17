use autharie_auth::Identity;
use autharie_core::{
    offers::{Offer, OrganisationOffer, ports::OrganisationOfferService},
    organisation::OrganisationId,
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/offers")]
pub struct TenantOffersRoute {
    pub organisation_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/platform/organisations/{organisation_id}/offers/{offer}")]
pub struct TenantOfferRoute {
    pub organisation_id: Uuid,
    pub offer: String,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantOfferView {
    pub offer: Offer,
    pub plan_default: bool,
    #[serde(rename = "override")]
    pub organisation_override: Option<bool>,
    pub effective: bool,
}

impl From<OrganisationOffer> for TenantOfferView {
    fn from(view: OrganisationOffer) -> Self {
        Self {
            offer: view.offer,
            plan_default: view.plan_default,
            organisation_override: view.organisation_override,
            effective: view.effective,
        }
    }
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantOffersResponse {
    pub data: Vec<TenantOfferView>,
}

#[derive(Serialize, ToSchema, PartialEq)]
pub struct TenantOfferResponse {
    pub data: TenantOfferView,
}

#[derive(Deserialize, ToSchema)]
pub struct SetTenantOfferRequest {
    /// `true` opens the offer, `false` closes it, `null` returns it to the plan.
    #[serde(deserialize_with = "Option::deserialize")]
    pub open: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/organisations/{organisation_id}/offers",
    summary = "list the offers of an organisation",
    tag = "platform",
    description = "Every offer with whether the organisation's plan opens it, the override staff \
                   set for this organisation when there is one, and whether it is open now.",
    params(TenantOffersRoute),
    responses(
        (status = 200, description = "Every offer as this organisation sees it", body = TenantOffersResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the view_estate right", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_tenant_offers_handler(
    TenantOffersRoute { organisation_id }: TenantOffersRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<TenantOffersResponse>, ApiError> {
    let offers = state
        .service
        .list_organisation_offers(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(TenantOffersResponse {
        data: offers.into_iter().map(TenantOfferView::from).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/organisations/{organisation_id}/offers/{offer}",
    summary = "open or close an offer for an organisation",
    tag = "platform",
    description = "`true` opens the offer and `false` closes it, whatever the plan says; `null` \
                   clears the override. Deployments already created on the offer stay.",
    params(TenantOfferRoute),
    request_body = SetTenantOfferRequest,
    responses(
        (status = 200, description = "The offer as the organisation now sees it", body = TenantOfferResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "This needs the act_on_tenant right", body = ApiError),
        (status = 404, description = "No such organisation or offer", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_tenant_offer_handler(
    TenantOfferRoute {
        organisation_id,
        offer,
    }: TenantOfferRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<SetTenantOfferRequest>,
) -> Result<Response<TenantOfferResponse>, ApiError> {
    let offer = offer.parse::<Offer>().map_err(|error| ApiError::NotFound {
        reason: error.to_string(),
    })?;

    let view = state
        .service
        .set_organisation_offer(
            identity,
            OrganisationId(organisation_id),
            offer,
            request.open,
        )
        .await?;

    Ok(Response::OK(TenantOfferResponse { data: view.into() }))
}
