use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{CoreError, billing::ports::BillingSuspensionRepository, organisation::OrganisationId};

use super::{
    Deployment, DeploymentId,
    ports::{DeploymentPolicy, DeploymentRepository},
};

/// What an installation offers somebody who runs a deployment: powering it
/// down without deleting it, and up again.
pub trait DeploymentPowerService: Send + Sync {
    fn stop_deployment(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Deployment, CoreError>> + Send;

    fn start_deployment(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Deployment, CoreError>> + Send;
}

const SUSPENDED_BY_BILLING: &str =
    "billing has stopped this organisation's deployments until its invoice is paid";

/// Powering a deployment down and up, by somebody who runs it.
///
/// Billing does the same to every deployment of an organisation that has not
/// paid, and takes its own record of which. Starting is refused for the whole
/// time that record is open, whichever of the two stopped a deployment: it
/// would run what billing stopped, and the person who asked would not be told
/// why it stopped again.
///
/// Billing itself is untouched: a stopped deployment keeps its open
/// subscription, so it is billed while stopped, as one stopped by billing is.
pub struct DeploymentPower<D, S, P> {
    deployments: D,
    suspensions: S,
    policy: P,
}

impl<D, S, P> DeploymentPower<D, S, P>
where
    D: DeploymentRepository,
    S: BillingSuspensionRepository,
    P: DeploymentPolicy,
{
    pub fn new(deployments: D, suspensions: S, policy: P) -> Self {
        Self {
            deployments,
            suspensions,
            policy,
        }
    }

    pub async fn stop(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        at: DateTime<Utc>,
    ) -> Result<Deployment, CoreError> {
        let mut deployment = self.owned(identity, organisation_id, deployment_id).await?;

        if !deployment.stop(at) {
            return Err(CoreError::DeploymentNotStoppable {
                deployment: deployment.id.0,
                status: deployment.status.to_string(),
            });
        }

        self.deployments.update(deployment.clone()).await?;

        Ok(deployment)
    }

    pub async fn start(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        at: DateTime<Utc>,
    ) -> Result<Deployment, CoreError> {
        let mut deployment = self.owned(identity, organisation_id, deployment_id).await?;

        if !deployment.start(at) {
            return Err(CoreError::DeploymentNotStopped {
                deployment: deployment.id.0,
                status: deployment.status.to_string(),
            });
        }

        if self
            .suspensions
            .find_active(&organisation_id)
            .await?
            .is_some()
        {
            return Err(CoreError::OrganisationSuspended {
                reason: SUSPENDED_BY_BILLING.to_string(),
            });
        }

        self.deployments.update(deployment.clone()).await?;

        Ok(deployment)
    }

    /// Checked before the read, so somebody who may not manage deployments
    /// cannot learn which ids exist from the difference between a refusal and
    /// a not-found.
    async fn owned(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        self.policy
            .can_manage_deployments(identity, organisation_id)
            .await?;

        self.deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })
    }
}

#[cfg(test)]
mod tests {
    use autharie_auth::User;
    use uuid::Uuid;

    use super::*;
    use crate::{
        billing::{
            ports::MockBillingSuspensionRepository,
            statement::StatementId,
            suspension::{BillingSuspension, BillingSuspensionId},
        },
        deployments::{DeploymentStatus, ports::MockDeploymentRepository},
    };

    struct Allowed;

    impl DeploymentPolicy for Allowed {
        async fn can_view_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn can_create_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn can_manage_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn can_delete_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }
    }

    struct Refused;

    impl DeploymentPolicy for Refused {
        async fn can_view_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Err(denied())
        }
        async fn can_create_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Err(denied())
        }
        async fn can_manage_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Err(denied())
        }
        async fn can_delete_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Err(denied())
        }
    }

    fn denied() -> CoreError {
        CoreError::PermissionDenied {
            reason: "no".to_string(),
        }
    }

    fn caller() -> Identity {
        Identity::User(User {
            id: "user-1".to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
            scopes: vec![],
            act: None,
        })
    }

    fn at() -> DateTime<Utc> {
        Utc::now()
    }

    fn deployment_in(organisation: OrganisationId, status: DeploymentStatus) -> Deployment {
        let mut deployment = crate::deployments::tests::deployment(status);
        deployment.organisation_id = organisation;
        deployment
    }

    fn holding(deployment: &Deployment, writes: usize) -> MockDeploymentRepository {
        let mut repository = MockDeploymentRepository::new();
        let held = deployment.clone();
        repository.expect_get_by_id().returning(move |_| {
            let held = held.clone();
            Box::pin(async move { Ok(Some(held)) })
        });
        repository
            .expect_update()
            .times(writes)
            .returning(|_| Box::pin(async { Ok(()) }));
        repository
    }

    fn billing_open(organisation: OrganisationId, open: bool) -> MockBillingSuspensionRepository {
        let mut suspensions = MockBillingSuspensionRepository::new();
        suspensions.expect_find_active().returning(move |_| {
            Box::pin(async move {
                Ok(open.then(|| BillingSuspension {
                    id: BillingSuspensionId::new(),
                    organisation_id: organisation,
                    statement_id: StatementId::new(),
                    stopped: Vec::new(),
                    suspended_organisation: false,
                    suspended_at: Utc::now(),
                    lifted_at: None,
                }))
            })
        });
        suspensions
    }

    fn organisation() -> OrganisationId {
        OrganisationId(Uuid::new_v4())
    }

    #[tokio::test]
    async fn a_running_deployment_is_stopped_and_recorded() {
        let organisation = organisation();
        let running = deployment_in(organisation, DeploymentStatus::Successful);
        let power = DeploymentPower::new(
            holding(&running, 1),
            billing_open(organisation, false),
            Allowed,
        );

        let stopped = power
            .stop(caller(), organisation, running.id, at())
            .await
            .expect("stopped");

        assert_eq!(stopped.status, DeploymentStatus::Stopped);
    }

    #[tokio::test]
    async fn a_stopped_deployment_is_started_and_comes_back_in_progress() {
        let organisation = organisation();
        let stopped = deployment_in(organisation, DeploymentStatus::Stopped);
        let power = DeploymentPower::new(
            holding(&stopped, 1),
            billing_open(organisation, false),
            Allowed,
        );

        let started = power
            .start(caller(), organisation, stopped.id, at())
            .await
            .expect("started");

        assert_eq!(started.status, DeploymentStatus::InProgress);
    }

    #[tokio::test]
    async fn stopping_what_cannot_be_stopped_names_its_state_and_writes_nothing() {
        for status in [
            DeploymentStatus::Stopped,
            DeploymentStatus::Pending,
            DeploymentStatus::Upgrading,
            DeploymentStatus::Deleting,
        ] {
            let organisation = organisation();
            let held = deployment_in(organisation, status.clone());
            let power = DeploymentPower::new(
                holding(&held, 0),
                billing_open(organisation, false),
                Allowed,
            );

            let refused = power
                .stop(caller(), organisation, held.id, at())
                .await
                .expect_err("refused");

            assert!(
                matches!(&refused, CoreError::DeploymentNotStoppable { status: named, .. } if *named == status.to_string()),
                "{status:?}: {refused:?}"
            );
        }
    }

    #[tokio::test]
    async fn starting_what_is_not_stopped_names_its_state_and_writes_nothing() {
        let organisation = organisation();
        let running = deployment_in(organisation, DeploymentStatus::Successful);
        let power = DeploymentPower::new(
            holding(&running, 0),
            billing_open(organisation, false),
            Allowed,
        );

        let refused = power
            .start(caller(), organisation, running.id, at())
            .await
            .expect_err("refused");

        assert!(matches!(refused, CoreError::DeploymentNotStopped { .. }));
    }

    #[tokio::test]
    async fn starting_is_refused_while_billing_has_the_organisation_suspended() {
        let organisation = organisation();
        let stopped = deployment_in(organisation, DeploymentStatus::Stopped);
        let power = DeploymentPower::new(
            holding(&stopped, 0),
            billing_open(organisation, true),
            Allowed,
        );

        let refused = power
            .start(caller(), organisation, stopped.id, at())
            .await
            .expect_err("refused");

        assert!(matches!(refused, CoreError::OrganisationSuspended { .. }));
    }

    #[tokio::test]
    async fn stopping_is_not_refused_while_billing_has_the_organisation_suspended() {
        let organisation = organisation();
        let running = deployment_in(organisation, DeploymentStatus::Successful);
        let power = DeploymentPower::new(
            holding(&running, 1),
            billing_open(organisation, true),
            Allowed,
        );

        power
            .stop(caller(), organisation, running.id, at())
            .await
            .expect("stopped");
    }

    #[tokio::test]
    async fn somebody_who_may_not_manage_deployments_is_refused_before_anything_is_read() {
        let organisation = organisation();
        let mut deployments = MockDeploymentRepository::new();
        deployments.expect_get_by_id().times(0);
        deployments.expect_update().times(0);
        let power = DeploymentPower::new(deployments, billing_open(organisation, false), Refused);

        let stop = power
            .stop(caller(), organisation, DeploymentId(Uuid::new_v4()), at())
            .await
            .expect_err("refused");
        let start = power
            .start(caller(), organisation, DeploymentId(Uuid::new_v4()), at())
            .await
            .expect_err("refused");

        assert!(matches!(stop, CoreError::PermissionDenied { .. }));
        assert!(matches!(start, CoreError::PermissionDenied { .. }));
    }

    #[tokio::test]
    async fn a_deployment_of_another_organisation_is_not_found() {
        let mine = organisation();
        let theirs = deployment_in(organisation(), DeploymentStatus::Successful);
        let power = DeploymentPower::new(holding(&theirs, 0), billing_open(mine, false), Allowed);

        let refused = power
            .stop(caller(), mine, theirs.id, at())
            .await
            .expect_err("not found");

        assert!(matches!(refused, CoreError::DeploymentNotFound { .. }));
    }
}
