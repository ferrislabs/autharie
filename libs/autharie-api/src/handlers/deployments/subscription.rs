use autharie_auth::Identity;
use autharie_core::{
    CoreError,
    billing::{Subscription, TierCode, ports::SubscriptionService},
    deployments::DeploymentId,
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
};
use axum::{Extension, Json, extract::State};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/subscription")]
pub struct SubscriptionRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangeSubscriptionRequest {
    #[schema(example = "standard-2026")]
    pub tier: String,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct SubscriptionResponse {
    pub tier: TierCode,
    pub monthly_price: i64,
    pub offer: Offer,
    pub features: Vec<IamFeature>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
}

impl From<&Subscription> for SubscriptionResponse {
    fn from(subscription: &Subscription) -> Self {
        let terms = subscription.terms();

        Self {
            tier: terms.tier.clone(),
            monthly_price: terms.monthly_price.minor(),
            offer: terms.offer,
            features: terms.features.iter().copied().collect(),
            starts_at: subscription.starts_at(),
            ends_at: subscription.ends_at(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/subscription",
    summary = "read the subscription of a deployment",
    tag = "deployments",
    description = "The tier the deployment is on, what it costs per month and the features it \
                   opens. `ends_at` is null while the subscription is running.",
    params(SubscriptionRoute),
    responses(
        (status = 200, description = "The subscription", body = SubscriptionResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's deployments", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment has no open subscription", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_subscription_handler(
    SubscriptionRoute {
        organisation_id,
        deployment_id,
    }: SubscriptionRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<SubscriptionResponse>, ApiError> {
    let subscription = state
        .service
        .subscription(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(SubscriptionResponse::from(&subscription)))
}

#[utoipa::path(
    put,
    path = "/{organisation_id}/deployments/{deployment_id}/subscription",
    summary = "move a deployment to another tier",
    tag = "deployments",
    description = "Closes the current subscription and opens one on the tier, from now. A paid \
                   tier needs a billing account with a valid payment method. Answering with the \
                   current subscription is what asking for the tier the deployment is already on \
                   does. The deployment then runs at the size of the new tier, in the same \
                   transaction: its data plane is told, and storage never shrinks. Moving \
                   between a shared and a managed database, between a shared and a dedicated \
                   data plane, or resizing a deployment that does not run on a shared data \
                   plane is not available yet. A refusal changes nothing.",
    params(SubscriptionRoute),
    request_body = ChangeSubscriptionRequest,
    responses(
        (status = 200, description = "The subscription now in force", body = SubscriptionResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not manage this organisation's billing", body = ApiError),
        (status = 404, description = "No such deployment, or no such tier", body = ApiError),
        (status = 409, description = "The billing account is not complete, the deployment has no open subscription, or the deployment cannot follow the tier: it is being upgraded, its data plane has no room for the tier's size, or the move is not available yet", body = ApiError),
        (status = 422, description = "The tier cannot be selected, or does not fit the deployment", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_subscription_handler(
    SubscriptionRoute {
        organisation_id,
        deployment_id,
    }: SubscriptionRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<ChangeSubscriptionRequest>,
) -> Result<Response<SubscriptionResponse>, ApiError> {
    let tier = TierCode::parse(&request.tier).map_err(CoreError::from)?;

    let subscription = state
        .service
        .change_subscription(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
            tier,
        )
        .await?;

    Ok(Response::OK(SubscriptionResponse::from(&subscription)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use autharie_core::{
        billing::{Money, SubscriptionId, SubscriptionParts, SubscriptionTerms},
        offers::Offer,
    };
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, day, 0, 0, 0)
            .single()
            .expect("a date")
    }

    fn subscription(ends_at: Option<DateTime<Utc>>) -> Subscription {
        Subscription::try_from(SubscriptionParts {
            id: SubscriptionId::new(),
            organisation_id: OrganisationId::new(),
            deployment_id: DeploymentId(Uuid::nil()),
            terms: SubscriptionTerms {
                tier: TierCode::parse("standard-2026").expect("a code"),
                monthly_price: Money::from_minor(4900),
                offer: Offer::Standard,
                features: BTreeSet::from([IamFeature::Mfa, IamFeature::Branding]),
            },
            starts_at: at(1),
            ends_at,
        })
        .expect("a subscription")
    }

    #[test]
    fn the_answer_is_the_terms_and_the_dates() {
        let answer = serde_json::to_value(SubscriptionResponse::from(&subscription(None)))
            .expect("a serializable answer");

        assert_eq!(
            answer,
            json!({
                "tier": "standard-2026",
                "monthly_price": 4900,
                "offer": "standard",
                "features": ["mfa", "branding"],
                "starts_at": "2026-12-01T00:00:00Z",
                "ends_at": null
            })
        );
    }

    #[test]
    fn a_closed_subscription_says_when_it_ended() {
        let answer = SubscriptionResponse::from(&subscription(Some(at(9))));

        assert_eq!(answer.ends_at, Some(at(9)));
    }

    #[test]
    fn the_answer_names_no_organisation_and_no_subscription_id() {
        let answer = serde_json::to_string(&SubscriptionResponse::from(&subscription(None)))
            .expect("a serializable answer");

        assert!(!answer.contains("organisation"), "{answer}");
        assert!(!answer.contains("\"id\""), "{answer}");
    }
}
