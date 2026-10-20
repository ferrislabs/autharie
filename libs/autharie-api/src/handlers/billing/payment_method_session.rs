use autharie_auth::Identity;
use autharie_core::{CoreError, billing::BillingError, organisation::OrganisationId};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/billing/payment-method-session")]
pub struct PaymentMethodSessionRoute {
    pub organisation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct PaymentMethodSessionResponse {
    pub url: String,
}

#[utoipa::path(
    post,
    path = "/{organisation_id}/billing/payment-method-session",
    summary = "open a session to add a payment method",
    tag = "billing",
    description = "Answers the address of the payment provider's page where the customer enters \
                   a card. The organisation needs a billing account first. The customer comes \
                   back to the console afterwards, and the payment method counts once the \
                   provider confirms it.",
    params(PaymentMethodSessionRoute),
    responses(
        (status = 200, description = "Where to send the customer", body = PaymentMethodSessionResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage this organisation's billing", body = ApiError),
        (status = 404, description = "No such organisation, or it has no billing account yet", body = ApiError),
        (status = 502, description = "The payment provider failed", body = ApiError),
        (status = 503, description = "Billing is not enabled on this installation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn payment_method_session_handler(
    PaymentMethodSessionRoute { organisation_id }: PaymentMethodSessionRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<PaymentMethodSessionResponse>, ApiError> {
    let runtime = state
        .billing
        .as_deref()
        .ok_or(CoreError::Billing(BillingError::NotConfigured))?;

    let url = state
        .service
        .payment_method_session(
            identity,
            OrganisationId(organisation_id),
            &runtime.provider,
            &runtime.return_url,
        )
        .await?;

    Ok(Response::OK(PaymentMethodSessionResponse { url }))
}
