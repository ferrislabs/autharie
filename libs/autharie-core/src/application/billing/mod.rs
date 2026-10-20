use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    billing::{
        BillingError, Subscription, Tier, TierCode,
        lifecycle::SubscriptionLifecycle,
        ports::{BillingPolicy, SubscriptionRepository, SubscriptionService, TierRepository},
    },
    deployments::{
        Deployment, DeploymentId,
        ports::{DeploymentPolicy, DeploymentRepository},
    },
    organisation::{OrganisationId, ports::OrganisationRepository},
};
use autharie_macros::transactional;

use crate::{AutharieService, infrastructure::role::permissions_in, policy::AuthariePolicy};

async fn live_deployment<D: DeploymentRepository>(
    deployments: &D,
    organisation_id: OrganisationId,
    deployment_id: DeploymentId,
) -> Result<Deployment, CoreError> {
    deployments
        .get_by_id(deployment_id)
        .await?
        .filter(|deployment| {
            deployment.organisation_id == organisation_id && deployment.deleted_at.is_none()
        })
        .ok_or(CoreError::DeploymentNotFound {
            id: deployment_id.0,
        })
}

impl SubscriptionService for AutharieService {
    #[transactional(tier)]
    async fn list_tiers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<Tier>, CoreError> {
        DeploymentPolicy::can_view_deployments(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity,
            organisation_id,
        )
        .await?;

        Ok(tier_repository
            .list()
            .await?
            .into_iter()
            .filter(|tier| tier.selectable)
            .collect())
    }

    #[transactional(deployment, subscription)]
    async fn subscription(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Subscription, CoreError> {
        DeploymentPolicy::can_view_deployments(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity,
            organisation_id,
        )
        .await?;

        let deployment =
            live_deployment(&deployment_repository, organisation_id, deployment_id).await?;

        Ok(subscription_repository
            .latest_for(&deployment.id)
            .await?
            .ok_or(BillingError::NoOpenSubscription {
                deployment: deployment.id.0,
            })?)
    }

    #[transactional(deployment, organisation, tier, subscription, billing_account)]
    async fn change_subscription(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        tier: TierCode,
    ) -> Result<Subscription, CoreError> {
        BillingPolicy::can_manage_billing(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity.clone(),
            organisation_id,
        )
        .await?;

        let deployment =
            live_deployment(&deployment_repository, organisation_id, deployment_id).await?;

        let organisation = organisation_repository
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })?;

        SubscriptionLifecycle::new(
            tier_repository,
            subscription_repository,
            billing_account_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .change(
            &identity,
            &organisation,
            &deployment,
            &tier,
            chrono::Utc::now(),
        )
        .await
    }
}
