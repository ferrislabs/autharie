use std::time::Duration;

use autharie_domain::{
    CoreError,
    dataplane::{
        entities::DataPlane,
        ports::HeraldIdentityProvisioner,
        provisioner::{ClusterProvisioner, ProvisionRequest, ProvisionTarget},
        value_objects::DataPlaneStatus,
    },
};
use futures::future::join_all;
use tracing::{error, info, warn};

use super::ports::{ClaimedCluster, CustomerClusterQueue};

const GENERIC_FAILURE: &str = "the cluster could not be created because of an internal error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerSettings {
    pub claim_lease: Duration,
    pub batch: u32,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            claim_lease: Duration::from_secs(45 * 60),
            batch: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProvisionReport {
    pub provisioned: u32,
    pub failed: u32,
    pub deferred: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeardownReport {
    pub released: u32,
    pub retrying: u32,
}

enum Outcome {
    Provisioned,
    Failed,
    Deferred,
}

pub struct CustomerClusterWorker<Q, P, I> {
    queue: Q,
    provisioner: P,
    identities: I,
    settings: WorkerSettings,
}

impl<Q, P, I> CustomerClusterWorker<Q, P, I>
where
    Q: CustomerClusterQueue,
    P: ClusterProvisioner,
    I: HeraldIdentityProvisioner,
{
    pub fn new(queue: Q, provisioner: P, identities: I, settings: WorkerSettings) -> Self {
        Self {
            queue,
            provisioner,
            identities,
            settings,
        }
    }

    pub async fn provision_pending(&self) -> Result<ProvisionReport, CoreError> {
        let claimed = self
            .queue
            .claim(self.settings.claim_lease, self.settings.batch)
            .await?;

        let outcomes = join_all(
            claimed
                .into_iter()
                .map(|cluster| self.provision_one(cluster)),
        )
        .await;

        let mut report = ProvisionReport::default();
        for outcome in outcomes {
            match outcome {
                Outcome::Provisioned => report.provisioned += 1,
                Outcome::Failed => report.failed += 1,
                Outcome::Deferred => report.deferred += 1,
            }
        }
        Ok(report)
    }

    pub async fn teardown_released(&self) -> Result<TeardownReport, CoreError> {
        let candidates = self.queue.teardown_candidates(self.settings.batch).await?;

        let mut report = TeardownReport::default();
        for data_plane in candidates {
            match self.tear_down(&data_plane).await {
                Ok(()) => report.released += 1,
                Err(error) => {
                    error!(
                        data_plane_id = %data_plane.id,
                        %error,
                        "tearing a customer cluster down failed, it is retried on the next pass"
                    );
                    report.retrying += 1;
                }
            }
        }
        Ok(report)
    }

    async fn provision_one(&self, claimed: ClaimedCluster) -> Outcome {
        let ClaimedCluster {
            mut data_plane,
            deployment_id,
            credential_id,
            profile,
            resources,
        } = claimed;
        let id = data_plane.id;

        if let Err(error) = self.provisioner.deprovision(&id).await {
            error!(
                data_plane_id = %id,
                %error,
                "leftovers of an earlier attempt could not be released, provisioning waits for the claim to expire"
            );
            return Outcome::Deferred;
        }

        info!(data_plane_id = %id, "provisioning a customer cluster");
        let request = ProvisionRequest {
            data_plane_id: id,
            organisation_id: match data_plane.allocation.owner() {
                Some(organisation_id) => organisation_id,
                None => {
                    error!(data_plane_id = %id, "a claimed data plane has no organisation");
                    return Outcome::Deferred;
                }
            },
            region: data_plane.region.clone(),
            minimum: resources,
            target: ProvisionTarget::Customer {
                credential_id,
                profile,
                deployment_id,
            },
        };

        match self.provisioner.provision(request).await {
            Ok(cluster) => {
                if !data_plane.provisioned(cluster) {
                    warn!(data_plane_id = %id, "the data plane left provisioning while its cluster was built");
                    return Outcome::Deferred;
                }
                match self.queue.complete(&data_plane).await {
                    Ok(true) => {
                        info!(data_plane_id = %id, "customer cluster provisioned, waiting for its first heartbeat");
                        Outcome::Provisioned
                    }
                    Ok(false) => {
                        warn!(data_plane_id = %id, "the data plane was failed or disabled while its cluster was built, the cluster is left to teardown");
                        Outcome::Deferred
                    }
                    Err(error) => {
                        error!(data_plane_id = %id, %error, "a provisioned cluster could not be recorded, it is rebuilt when the claim expires");
                        Outcome::Deferred
                    }
                }
            }
            Err(error) => {
                let reason = readable_reason(&error);
                warn!(data_plane_id = %id, %error, "provisioning a customer cluster failed");
                match self.queue.fail(&id, &reason).await {
                    Ok(_) => Outcome::Failed,
                    Err(failure) => {
                        error!(data_plane_id = %id, error = %failure, "a failed provisioning could not be recorded");
                        Outcome::Deferred
                    }
                }
            }
        }
    }

    async fn tear_down(&self, data_plane: &DataPlane) -> Result<(), CoreError> {
        let id = data_plane.id;

        if data_plane.herald.is_some() {
            self.identities.revoke(id).await?;
        }
        self.provisioner.deprovision(&id).await?;

        if data_plane.status != DataPlaneStatus::Failed {
            self.queue.disable(&id).await?;
        }
        info!(data_plane_id = %id, "customer cluster released");
        Ok(())
    }
}

fn readable_reason(error: &CoreError) -> String {
    match error {
        CoreError::Provision(error) => error.to_string(),
        CoreError::Credential(error) => error.to_string(),
        CoreError::Profile(error) => error.to_string(),
        _ => GENERIC_FAILURE.to_string(),
    }
}
