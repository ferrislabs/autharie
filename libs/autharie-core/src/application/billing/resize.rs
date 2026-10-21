use autharie_domain::{
    ArchiveConfig, CoreError,
    action::ports::ActionRepository,
    backups::ports::BackupScheduleRepository,
    dataplane::ports::DataPlaneRepository,
    deployments::{
        Deployment,
        ports::DeploymentRepository,
        resize::{Resize, plan_resize},
    },
    offers::Offer,
};
use chrono::{DateTime, Utc};
use tracing::info;

use super::announce::Announcer;

pub(crate) struct TierResize<'a, P, D, B, A>
where
    A: ActionRepository,
{
    planes: P,
    deployments: D,
    announcer: Announcer<'a, B, A>,
    now: DateTime<Utc>,
}

impl<'a, P, D, B, A> TierResize<'a, P, D, B, A>
where
    P: DataPlaneRepository,
    D: DeploymentRepository,
    B: BackupScheduleRepository,
    A: ActionRepository,
{
    pub(crate) fn new(
        planes: P,
        deployments: D,
        schedules: B,
        actions: A,
        archive: &'a ArchiveConfig,
        domain: Option<&'a str>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            planes,
            deployments,
            announcer: Announcer::new(schedules, actions, archive, domain),
            now,
        }
    }

    pub(crate) async fn follow(
        &self,
        mut deployment: Deployment,
        offer: Offer,
    ) -> Result<(), CoreError> {
        let resize = plan_resize(&self.planes, &deployment, offer).await?;

        if resize == Resize::Unchanged {
            return Ok(());
        }

        deployment.apply_resize(offer, resize, self.now);

        if matches!(resize, Resize::Resources(_)) {
            self.announcer
                .announce(&deployment, "deployment.update")
                .await?;
        }

        let id = deployment.id;
        self.deployments.update(deployment).await?;

        info!(deployment = %id, %offer, "a tier change resized a deployment");

        Ok(())
    }
}
