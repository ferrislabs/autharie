use autharie_auth::Identity;
use autharie_domain::deployments::{
    DeploymentId,
    reachability_history::{
        DeploymentDowntime, DeploymentUptime, ReachabilityCheck, ReachabilityCheckRepository,
        service::ReachabilityServiceImpl,
    },
};
use autharie_macros::transactional;
use chrono::{DateTime, Duration, Utc};

use crate::{AutharieService, CoreError, policy::PlatformRightsPolicy};

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
