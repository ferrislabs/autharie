use autharie_domain::{
    ArchiveConfig, CoreError,
    action::{
        ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    backups::ports::BackupScheduleRepository,
    deployments::Deployment,
};

use crate::application::deployment::{archive_section, deployment_hostname, deployment_payload};

pub(crate) struct Announcer<'a, B, A>
where
    A: ActionRepository,
{
    schedules: B,
    actions: ActionServiceImpl<A>,
    archive: &'a ArchiveConfig,
    domain: Option<&'a str>,
}

impl<'a, B, A> Announcer<'a, B, A>
where
    B: BackupScheduleRepository,
    A: ActionRepository,
{
    pub(crate) fn new(
        schedules: B,
        actions: A,
        archive: &'a ArchiveConfig,
        domain: Option<&'a str>,
    ) -> Self {
        Self {
            schedules,
            actions: ActionServiceImpl::new(actions),
            archive,
            domain,
        }
    }

    pub(crate) async fn announce(
        &self,
        deployment: &Deployment,
        action: &str,
    ) -> Result<(), CoreError> {
        self.announce_as(deployment, action, ActionSource::System)
            .await
    }

    pub(crate) async fn announce_as(
        &self,
        deployment: &Deployment,
        action: &str,
        source: ActionSource,
    ) -> Result<(), CoreError> {
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
                source,
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
