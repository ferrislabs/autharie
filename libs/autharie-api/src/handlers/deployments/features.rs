use autharie_auth::Identity;
use autharie_core::{
    billing::{SubscriptionTerms, TierCode},
    deployments::DeploymentId,
    organisation::{OrganisationId, features::IamFeature},
};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/deployments/{deployment_id}/features")]
pub struct DeploymentFeaturesRoute {
    pub organisation_id: Uuid,
    pub deployment_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct DeploymentFeature {
    pub feature: IamFeature,
    pub open: bool,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct DeploymentFeaturesResponse {
    pub tier: TierCode,
    pub features: Vec<DeploymentFeature>,
}

impl From<&SubscriptionTerms> for DeploymentFeaturesResponse {
    fn from(terms: &SubscriptionTerms) -> Self {
        Self {
            tier: terms.tier.clone(),
            features: IamFeature::ALL
                .into_iter()
                .map(|feature| DeploymentFeature {
                    feature,
                    open: terms.opens(feature),
                })
                .collect(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/deployments/{deployment_id}/features",
    summary = "list the IAM features a deployment's tier opens",
    tag = "deployments",
    description = "Every feature there is, each saying whether the tier this deployment is \
                   subscribed to opens it.",
    params(DeploymentFeaturesRoute),
    responses(
        (status = 200, description = "What the deployment's tier opens", body = DeploymentFeaturesResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's deployments", body = ApiError),
        (status = 404, description = "No such deployment", body = ApiError),
        (status = 409, description = "The deployment has no open subscription", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_deployment_features_handler(
    DeploymentFeaturesRoute {
        organisation_id,
        deployment_id,
    }: DeploymentFeaturesRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<DeploymentFeaturesResponse>, ApiError> {
    let terms = state
        .service
        .deployment_features(
            identity,
            OrganisationId(organisation_id),
            DeploymentId(deployment_id),
        )
        .await?;

    Ok(Response::OK(DeploymentFeaturesResponse::from(&terms)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use autharie_core::{billing::Money, offers::Offer};

    use super::*;

    fn terms(open: &[IamFeature]) -> SubscriptionTerms {
        SubscriptionTerms {
            tier: TierCode::parse("standard-2026").expect("a code"),
            monthly_price: Money::from_minor(4900),
            offer: Offer::Standard,
            features: open.iter().copied().collect::<BTreeSet<_>>(),
        }
    }

    #[test]
    fn every_feature_is_listed_in_order_whether_open_or_not() {
        let answer =
            DeploymentFeaturesResponse::from(&terms(&[IamFeature::Branding, IamFeature::Mfa]));

        let listed: Vec<IamFeature> = answer.features.iter().map(|entry| entry.feature).collect();
        assert_eq!(listed, IamFeature::ALL);
        let open: Vec<IamFeature> = answer
            .features
            .iter()
            .filter(|entry| entry.open)
            .map(|entry| entry.feature)
            .collect();
        assert_eq!(open, vec![IamFeature::Mfa, IamFeature::Branding]);
        assert_eq!(answer.tier.as_str(), "standard-2026");
    }

    #[test]
    fn a_tier_opening_nothing_lists_everything_closed() {
        let answer = DeploymentFeaturesResponse::from(&terms(&[]));

        assert_eq!(answer.features.len(), IamFeature::ALL.len());
        assert!(answer.features.iter().all(|entry| !entry.open));
    }
}
