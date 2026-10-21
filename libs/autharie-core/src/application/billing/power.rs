use autharie_domain::{
    ArchiveConfig, CoreError,
    action::ports::ActionRepository,
    backups::ports::BackupScheduleRepository,
    billing::ports::{DeploymentPower, SubscriptionRepository},
    deployments::{Deployment, DeploymentId, ports::DeploymentRepository},
    organisation::OrganisationId,
};
use chrono::{DateTime, Utc};
use tracing::info;

use super::announce::Announcer;

pub(crate) struct DeploymentSwitch<'a, S, D, B, A>
where
    A: ActionRepository,
{
    subscriptions: S,
    deployments: D,
    announcer: Announcer<'a, B, A>,
    now: DateTime<Utc>,
}

impl<'a, S, D, B, A> DeploymentSwitch<'a, S, D, B, A>
where
    S: SubscriptionRepository,
    D: DeploymentRepository,
    B: BackupScheduleRepository,
    A: ActionRepository,
{
    pub(crate) fn new(
        subscriptions: S,
        deployments: D,
        schedules: B,
        actions: A,
        archive: &'a ArchiveConfig,
        domain: Option<&'a str>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            subscriptions,
            deployments,
            announcer: Announcer::new(schedules, actions, archive, domain),
            now,
        }
    }

    async fn switch(
        &self,
        deployment_id: DeploymentId,
        transition: impl FnOnce(&mut Deployment, DateTime<Utc>) -> bool + Send,
        action: &str,
    ) -> Result<bool, CoreError> {
        let Some(mut deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };

        if !transition(&mut deployment, self.now) {
            return Ok(false);
        }

        self.announcer.announce(&deployment, action).await?;
        self.deployments.update(deployment).await?;

        info!(deployment = %deployment_id, action, "billing switched a deployment");

        Ok(true)
    }
}

impl<S, D, B, A> DeploymentPower for DeploymentSwitch<'_, S, D, B, A>
where
    S: SubscriptionRepository,
    D: DeploymentRepository,
    B: BackupScheduleRepository,
    A: ActionRepository,
{
    async fn stop_billable(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Vec<DeploymentId>, CoreError> {
        let mut stopped = Vec::new();

        for subscription in self
            .subscriptions
            .list_open_billable(organisation_id)
            .await?
        {
            let deployment_id = subscription.deployment_id();

            if self
                .switch(deployment_id, Deployment::stop, "deployment.stop")
                .await?
            {
                stopped.push(deployment_id);
            }
        }

        Ok(stopped)
    }

    async fn start(&self, deployment_ids: &[DeploymentId]) -> Result<Vec<DeploymentId>, CoreError> {
        let mut started = Vec::new();

        for &deployment_id in deployment_ids {
            if self
                .switch(deployment_id, Deployment::start, "deployment.start")
                .await?
            {
                started.push(deployment_id);
            }
        }

        Ok(started)
    }
}
