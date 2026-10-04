use std::time::Duration;

use autharie_domain::{
    CoreError,
    dataplane::{
        cluster_profile::ClusterProfile,
        credential::CloudCredentialId,
        entities::DataPlane,
        value_objects::{DataPlaneId, DeploymentResources},
    },
    deployments::DeploymentId,
};

#[derive(Debug, Clone)]
pub struct ClaimedCluster {
    pub data_plane: DataPlane,
    pub deployment_id: DeploymentId,
    pub credential_id: CloudCredentialId,
    pub profile: ClusterProfile,
    pub resources: DeploymentResources,
}

pub trait CustomerClusterQueue: Send + Sync {
    fn claim(
        &self,
        lease: Duration,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ClaimedCluster>, CoreError>> + Send;

    fn complete(
        &self,
        data_plane: &DataPlane,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn fail(
        &self,
        id: &DataPlaneId,
        reason: &str,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn teardown_candidates(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<DataPlane>, CoreError>> + Send;

    fn disable(&self, id: &DataPlaneId) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
