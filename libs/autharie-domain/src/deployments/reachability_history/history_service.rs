use autharie_auth::Identity;
use chrono::{Duration, Utc};

use crate::{
    CoreError,
    deployments::{
        DeploymentId,
        ports::{DeploymentPolicy, DeploymentRepository},
        reachability_history::{
            AvailabilityHistory, ReachabilityCheckRepository, availability_history,
        },
    },
    organisation::OrganisationId,
};

pub struct AvailabilityHistoryServiceImpl<R, D, P>
where
    R: ReachabilityCheckRepository,
    D: DeploymentRepository,
    P: DeploymentPolicy,
{
    repository: R,
    deployment_repository: D,
    policy: P,
}

impl<R, D, P> AvailabilityHistoryServiceImpl<R, D, P>
where
    R: ReachabilityCheckRepository,
    D: DeploymentRepository,
    P: DeploymentPolicy,
{
    pub fn new(repository: R, deployment_repository: D, policy: P) -> Self {
        Self {
            repository,
            deployment_repository,
            policy,
        }
    }

    pub async fn history(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        days: u32,
    ) -> Result<AvailabilityHistory, CoreError> {
        // Before the read, so somebody who may not look here cannot learn
        // which ids exist from the difference between a refusal and a
        // not-found.
        self.policy
            .can_view_deployments(identity, organisation_id)
            .await?;

        self.deployment_repository
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .filter(|deployment| deployment.deleted_at.is_none())
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })?;

        let today = Utc::now().date_naive();
        let first_day = today - Duration::days(i64::from(days) - 1);
        let counts = self
            .repository
            .daily_check_counts(deployment_id, first_day)
            .await?;

        Ok(availability_history(&counts, today, days))
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        dataplane::value_objects::{DataPlaneId, DeploymentResources},
        deployments::{
            Deployment, DeploymentKind, DeploymentName, DeploymentStatus,
            ports::MockDeploymentRepository,
            reachability_history::{DailyCheckCount, MockReachabilityCheckRepository},
        },
        user::UserId,
        version::Version,
    };

    struct Policy(bool);

    impl DeploymentPolicy for Policy {
        async fn can_view_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            if self.0 {
                Ok(())
            } else {
                Err(CoreError::PermissionDenied {
                    reason: "insufficient permissions".to_string(),
                })
            }
        }

        async fn can_create_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            unreachable!("history is only read")
        }

        async fn can_manage_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            unreachable!("history is only read")
        }

        async fn can_delete_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            unreachable!("history is only read")
        }
    }

    fn caller() -> Identity {
        Identity::User(autharie_auth::User {
            id: "someone".to_string(),
            username: "someone".to_string(),
            email: None,
            name: None,
            roles: Vec::new(),
        })
    }

    fn deployment(organisation: Uuid) -> Deployment {
        let at = Utc::now();
        Deployment {
            topology: Default::default(),
            id: DeploymentId(Uuid::from_u128(1)),
            organisation_id: OrganisationId(organisation),
            dataplane_id: DataPlaneId(Uuid::from_u128(9)),
            name: DeploymentName("auth".to_string()),
            kind: DeploymentKind::Ferriskey,
            version: Version::new(26, 0, 0),
            status: DeploymentStatus::Successful,
            namespace: "tenant-a".to_string(),
            environment: crate::deployments::environment::Environment::Development,
            offer: None,
            restored_from: None,
            resources: DeploymentResources::DEFAULT,
            created_by: UserId(Uuid::from_u128(2)),
            created_at: at,
            updated_at: at,
            deployed_at: None,
            deleted_at: None,
            auto_upgrade: Default::default(),
            maintenance_window: None,
            network_access: crate::deployments::network::NetworkAccess::Open,
            last_verified_restore_at: None,
            last_restore_drill_seconds: None,
            log_shipping_enabled: false,
            iam_settings: Default::default(),
            distribution: Default::default(),
        }
    }

    fn deployments(found: Option<Deployment>) -> MockDeploymentRepository {
        let mut mock = MockDeploymentRepository::new();
        mock.expect_get_by_id().returning(move |_| {
            let found = found.clone();
            Box::pin(async move { Ok(found) })
        });
        mock
    }

    const ORGANISATION: Uuid = Uuid::from_u128(5);

    #[tokio::test]
    async fn a_member_reads_the_history_of_their_deployment() {
        let mut repository = MockReachabilityCheckRepository::new();
        repository
            .expect_daily_check_counts()
            .times(1)
            .returning(|_, _| {
                let today = Utc::now().date_naive();
                Box::pin(async move {
                    Ok(vec![DailyCheckCount {
                        day: today,
                        checks: 10,
                        failed: 1,
                    }])
                })
            });
        let service = AvailabilityHistoryServiceImpl::new(
            repository,
            deployments(Some(deployment(ORGANISATION))),
            Policy(true),
        );

        let history = service
            .history(
                caller(),
                OrganisationId(ORGANISATION),
                DeploymentId(Uuid::from_u128(1)),
                7,
            )
            .await
            .expect("a member");

        assert_eq!(history.days.len(), 7);
        assert_eq!(history.days[6].failed, 1);
        assert_eq!(history.uptime, Some(0.9));
    }

    #[tokio::test]
    async fn a_deployment_of_another_organisation_is_not_found() {
        let service = AvailabilityHistoryServiceImpl::new(
            MockReachabilityCheckRepository::new(),
            deployments(Some(deployment(Uuid::from_u128(6)))),
            Policy(true),
        );

        let result = service
            .history(
                caller(),
                OrganisationId(ORGANISATION),
                DeploymentId(Uuid::from_u128(1)),
                90,
            )
            .await;

        assert!(matches!(result, Err(CoreError::DeploymentNotFound { .. })));
    }

    #[tokio::test]
    async fn a_caller_who_may_not_view_is_refused_before_anything_is_read() {
        let service = AvailabilityHistoryServiceImpl::new(
            MockReachabilityCheckRepository::new(),
            MockDeploymentRepository::new(),
            Policy(false),
        );

        let result = service
            .history(
                caller(),
                OrganisationId(ORGANISATION),
                DeploymentId(Uuid::from_u128(1)),
                90,
            )
            .await;

        assert!(matches!(result, Err(CoreError::PermissionDenied { .. })));
    }
}
