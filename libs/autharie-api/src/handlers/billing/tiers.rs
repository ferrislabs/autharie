use autharie_auth::Identity;
use autharie_core::{
    billing::{
        BillingPeriod, FirstInvoice, Hosting, Tier, TierCode, first_invoice,
        ports::SubscriptionService,
    },
    deployments::DeploymentKind,
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

pub const CURRENCY: &str = "EUR";

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/billing/tiers")]
pub struct TiersRoute {
    pub organisation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct FirstInvoiceResponse {
    pub period: BillingPeriod,
    pub amount: i64,
}

impl From<FirstInvoice> for FirstInvoiceResponse {
    fn from(invoice: FirstInvoice) -> Self {
        Self {
            period: invoice.period,
            amount: invoice.amount.minor(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct TierResponse {
    pub code: TierCode,
    pub name: String,
    pub monthly_price: i64,
    #[schema(example = "EUR")]
    pub currency: &'static str,
    pub offer: Offer,
    pub features: Vec<IamFeature>,
    pub engines: Vec<DeploymentKind>,
    pub hostings: Vec<Hosting>,
    pub first_invoice: Option<FirstInvoiceResponse>,
}

impl TierResponse {
    pub fn of(tier: &Tier, opened_at: DateTime<Utc>, first_period: Option<BillingPeriod>) -> Self {
        Self {
            code: tier.code.clone(),
            name: tier.name.clone(),
            monthly_price: tier.monthly_price.minor(),
            currency: CURRENCY,
            offer: tier.offer,
            features: tier.features.iter().copied().collect(),
            engines: tier.engines.clone(),
            hostings: tier.hostings.clone(),
            first_invoice: first_period.map(|first_period| {
                first_invoice(tier.monthly_price, opened_at, first_period).into()
            }),
        }
    }
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct ListTiersResponse {
    pub data: Vec<TierResponse>,
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/billing/tiers",
    summary = "list the tiers a deployment can be subscribed to",
    tag = "billing",
    description = "The tiers on sale, cheapest first. A tier that is no longer sold, such as the \
                   one a deployment was grandfathered on, is not listed. `first_invoice` says \
                   what subscribing now would bill first and for which month, and is null when \
                   billing is not enabled on this installation.",
    params(TiersRoute),
    responses(
        (status = 200, description = "The tiers on sale", body = ListTiersResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's deployments", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_tiers_handler(
    TiersRoute { organisation_id }: TiersRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<ListTiersResponse>, ApiError> {
    let tiers = state
        .service
        .list_tiers(identity, OrganisationId(organisation_id))
        .await?;
    let now = Utc::now();
    let first_period = state.billing.as_deref().map(|runtime| runtime.first_period);

    Ok(Response::OK(ListTiersResponse {
        data: tiers
            .iter()
            .map(|tier| TierResponse::of(tier, now, first_period))
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use autharie_core::billing::Money;
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn tier() -> Tier {
        Tier {
            code: TierCode::parse("standard-2026").expect("a code"),
            name: "Standard".to_string(),
            monthly_price: Money::from_minor(4900),
            offer: Offer::Standard,
            features: BTreeSet::from([IamFeature::Mfa, IamFeature::Branding]),
            engines: vec![DeploymentKind::Ferriskey],
            hostings: vec![Hosting::Shared],
            selectable: true,
        }
    }

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, day, 0, 0, 0)
            .single()
            .expect("a date")
    }

    #[test]
    fn a_tier_is_priced_in_minor_units_of_euros() {
        let answer = serde_json::to_value(TierResponse::of(&tier(), at(1), None))
            .expect("a serializable tier");

        assert_eq!(
            answer,
            json!({
                "code": "standard-2026",
                "name": "Standard",
                "monthly_price": 4900,
                "currency": "EUR",
                "offer": "standard",
                "features": ["mfa", "branding"],
                "engines": ["ferriskey"],
                "hostings": ["shared"],
                "first_invoice": null
            })
        );
    }

    #[test]
    fn the_first_invoice_is_the_rest_of_the_month_when_billing_is_on() {
        let first_period = BillingPeriod::new(2026, 11).expect("a period");

        let answer = TierResponse::of(&tier(), at(16), Some(first_period));

        assert_eq!(
            answer.first_invoice,
            Some(FirstInvoiceResponse {
                period: BillingPeriod::new(2026, 12).expect("a period"),
                amount: 2529,
            })
        );
    }

    #[test]
    fn a_subscription_before_the_first_period_pays_that_period_in_full() {
        let first_period = BillingPeriod::new(2027, 2).expect("a period");

        let answer = TierResponse::of(&tier(), at(16), Some(first_period));

        assert_eq!(
            answer.first_invoice,
            Some(FirstInvoiceResponse {
                period: first_period,
                amount: 4900,
            })
        );
    }
}
