use crate::{
    CoreError,
    dataplane::{
        cluster_profile::ClusterProfile,
        credential::CloudCredentialId,
        herald_identity::HeraldBinding,
        value_objects::{Capacity, DataPlaneId, DeploymentResources, Region},
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};

/// Creates the infrastructure a data plane runs on.
///
/// This is the one place the pull model reverses. Everywhere else the data
/// plane claims work from the control plane; here the control plane must act
/// on a cluster that does not exist yet, because an empty cluster has no
/// Herald in it to do the claiming.
#[cfg_attr(test, mockall::automock)]
pub trait ClusterProvisioner: Send + Sync {
    fn provision(
        &self,
        request: ProvisionRequest,
    ) -> impl Future<Output = Result<ProvisionedCluster, CoreError>> + Send;

    /// Releases the infrastructure. Must be idempotent: it is called on
    /// cleanup paths that cannot know whether a previous attempt got through.
    fn deprovision(&self, id: &DataPlaneId) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Moves an existing cluster to another profile, keeping the data plane.
    fn resize(
        &self,
        id: &DataPlaneId,
        profile: &ClusterProfile,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub struct ProvisionRequest {
    pub data_plane_id: DataPlaneId,
    pub organisation_id: OrganisationId,
    pub region: Region,
    /// What the first deployment needs. A provisioner sizes the cluster to at
    /// least this, and is free to size it larger.
    pub minimum: DeploymentResources,
    pub target: ProvisionTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedCluster {
    pub herald: HeraldBinding,
    pub capacity: Capacity,
}

/// Whose infrastructure the cluster is made in.
pub enum ProvisionTarget {
    /// The platform's own account, as every provisioner did before customers
    /// brought their own.
    Platform,
    /// The customer's account. The adapter resolves the credential itself, so
    /// the secret never travels through the request.
    Customer {
        credential_id: CloudCredentialId,
        profile: ClusterProfile,
        deployment_id: DeploymentId,
    },
}

/// Why a cluster could not be created. Each one is the readable reason a
/// data plane shows when it ends `Failed`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvisionError {
    #[error("the provider quota in this account does not allow this cluster")]
    QuotaExceeded,

    #[error("the provider cannot create clusters in this region")]
    RegionUnavailable,

    #[error("the node pool never became ready")]
    NodePoolNeverConverged,

    #[error("the provider rejected the credential")]
    CredentialRejected,
}
