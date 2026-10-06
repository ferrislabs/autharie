//! What DNS reconciliation needs to know, read without an identity to check.
//!
//! Publishing a record is not something a caller asks for -- it follows from
//! a deployment existing and its data plane having an address, the same way
//! purging deleted deployments follows from a retention window rather than a
//! request. The reads below serve that: [`AutharieService::dataplane_gateway_address`]
//! answers "can this one deployment's record be written right now", asked
//! once at placement, and [`AutharieService::dns_targets`] answers "everything
//! that should have a record right now", asked on a sweep. Neither takes an
//! [`autharie_auth::Identity`] for the same reason `purge_deleted_deployments`
//! does not: nobody is the caller, the installation's own upkeep is.
//!
//! Turning what these return into an actual record is `autharie-ovh`'s job,
//! wired in at `autharie-api` -- this crate never holds a `DnsProvider`, the
//! same way it never held an object store or a key manager.

use autharie_domain::{
    CoreError,
    dataplane::{ports::DataPlaneRepository, value_objects::DataPlaneId},
    deployments::{Deployment, DeploymentStatus, ports::DeploymentRepository},
};
use autharie_macros::transactional;

use crate::AutharieService;

impl AutharieService {
    /// Where a data plane's own Gateway answers, if it has reported one yet.
    #[transactional(data_plane)]
    pub async fn dataplane_gateway_address(
        &self,
        dataplane_id: DataPlaneId,
    ) -> Result<Option<String>, CoreError> {
        Ok(data_plane_repository
            .find_by_id(&dataplane_id)
            .await?
            .and_then(|dataplane| dataplane.gateway_address))
    }

    /// Every deployment that should have a DNS record right now, paired with
    /// the address it should point at.
    ///
    /// A data plane with no known address yet contributes nothing here
    /// rather than a target with an empty address: there is no record to
    /// point at nowhere, only one not written yet. A deployment already
    /// tearing down is left out the same way -- its record was removed when
    /// deletion was asked for, and a sweep finding it again is not a reason
    /// to bring it back.
    #[transactional(data_plane, deployment)]
    pub async fn dns_targets(&self) -> Result<Vec<(Deployment, String)>, CoreError> {
        let dataplanes = data_plane_repository.list_all().await?;
        let mut targets = Vec::new();

        for dataplane in dataplanes {
            let Some(address) = dataplane.gateway_address else {
                continue;
            };

            let deployments = deployment_repository
                .list_by_dataplane(&dataplane.id)
                .await?;

            for deployment in deployments {
                if deployment.status == DeploymentStatus::Deleting {
                    continue;
                }

                targets.push((deployment, address.clone()));
            }
        }

        Ok(targets)
    }
}
