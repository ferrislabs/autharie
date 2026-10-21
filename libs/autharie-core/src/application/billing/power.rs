use autharie_domain::{
    ArchiveConfig, CoreError,
    action::{
        ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    backups::ports::BackupScheduleRepository,
    billing::ports::{DeploymentPower, SubscriptionRepository},
    deployments::{Deployment, DeploymentId, ports::DeploymentRepository},
    organisation::OrganisationId,
};
use chrono::{DateTime, Utc};
use tracing::info;

use crate::application::deployment::{archive_section, deployment_hostname, deployment_payload};

pub(crate) struct DeploymentSwitch<'a, S, D, B, A>
where
    A: ActionRepository,
{
    subscriptions: S,
    deployments: D,
    schedules: B,
    actions: ActionServiceImpl<A>,
    archive: &'a ArchiveConfig,
    domain: Option<&'a str>,
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
            schedules,
            actions: ActionServiceImpl::new(actions),
            archive,
            domain,
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

        self.announce(&deployment, action).await?;
        self.deployments.update(deployment).await?;

        info!(deployment = %deployment_id, action, "billing switched a deployment");

        Ok(true)
    }

    async fn announce(&self, deployment: &Deployment, action: &str) -> Result<(), CoreError> {
        // Genesis applies the whole desired state: a payload without the
        // hostname resets it, one without the archive drops the backups.
        let hostname = deployment_hostname(self.domain, deployment);
        let archive = self.archive_of(deployment).await?;

        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(action.to_string()),
                ActionTarget {
                    kind: TargetKind::Deployment,
                    id: deployment.id.0,
                },
                ActionPayload {
                    data: deployment_payload(deployment, archive, hostname),
                },
                ActionVersion(1),
                ActionSource::System,
            ))
            .await?;

        Ok(())
    }

    async fn archive_of(
        &self,
        deployment: &Deployment,
    ) -> Result<Option<serde_json::Value>, CoreError> {
        let Some(destination) = self
            .archive
            .destination_for(deployment.organisation_id, deployment.id)
        else {
            return Ok(None);
        };

        Ok(self
            .schedules
            .get(&deployment.id)
            .await?
            .map(|schedule| archive_section(&destination, &self.archive.encryption, &schedule)))
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
