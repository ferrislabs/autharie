use crate::{CoreError, dataplane::value_objects::DataPlaneId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Cluster,
    NodePool,
    PrivateNetwork,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cluster => "cluster",
            Self::NodePool => "node_pool",
            Self::PrivateNetwork => "private_network",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedResource {
    pub kind: ResourceKind,
    pub provider_id: String,
}

/// What a provisioner created for a data plane, written before the next
/// step runs so that teardown works from the record rather than from what
/// the adapter believes it did.
#[cfg_attr(test, mockall::automock)]
pub trait ClusterInventory: Send + Sync {
    fn record(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn unreleased(
        &self,
        data_plane_id: &DataPlaneId,
    ) -> impl Future<Output = Result<Vec<ProvisionedResource>, CoreError>> + Send;

    fn mark_released(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}
