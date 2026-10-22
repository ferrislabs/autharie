use autharie_auth::Identity;
use autharie_domain::deployments::{
    DeploymentId,
    reachability_history::{
        AvailabilityHistory, DeploymentDowntime, DeploymentUptime, ReachabilityCheck,
        ReachabilityCheckRepository, history_service::AvailabilityHistoryServiceImpl,
        service::ReachabilityServiceImpl,
    },
};
use autharie_domain::organisation::OrganisationId;
use autharie_macros::transactional;
use chrono::{DateTime, Duration, Utc};

use crate::{
    AutharieService, CoreError,
    infrastructure::role::permissions_in,
    policy::{AuthariePolicy, PlatformRightsPolicy},
};

impl AutharieService {
    #[transactional(reachability_check)]
    pub async fn record_reachability_check(
        &self,
        check: ReachabilityCheck,
    ) -> Result<(), CoreError> {
        reachability_check_repository.record_check(check).await
    }

    #[transactional(reachability_check)]
    pub async fn get_deployment_uptime(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
    ) -> Result<DeploymentUptime, CoreError> {
        let service = ReachabilityServiceImpl::new(
            reachability_check_repository,
            PlatformRightsPolicy::new(
                autharie_postgres::platform::PostgresOperatorRepository::new(&tx),
            ),
        );

        service.get_uptime(identity, deployment_id).await
    }

    #[transactional(reachability_check)]
    pub async fn get_deployment_downtime(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        since: DateTime<Utc>,
    ) -> Result<DeploymentDowntime, CoreError> {
        let service = ReachabilityServiceImpl::new(
            reachability_check_repository,
            PlatformRightsPolicy::new(
                autharie_postgres::platform::PostgresOperatorRepository::new(&tx),
            ),
        );

        service.get_downtime(identity, deployment_id, since).await
    }

    #[transactional(reachability_check, deployment)]
    pub async fn get_deployment_availability(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        days: u32,
    ) -> Result<AvailabilityHistory, CoreError> {
        AvailabilityHistoryServiceImpl::new(
            reachability_check_repository,
            deployment_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .history(identity, organisation_id, deployment_id, days)
        .await
    }

    #[transactional(reachability_check)]
    pub async fn purge_old_reachability_checks(
        &self,
        retention: Duration,
    ) -> Result<u64, CoreError> {
        reachability_check_repository
            .purge_old_checks(retention)
            .await
    }
}
