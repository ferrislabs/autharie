use crate::{
    CoreError,
    dataplane::{
        credential::SecretString,
        herald_identity::HeraldBinding,
        value_objects::{DataPlaneId, Region},
    },
    organisation::OrganisationId,
};

pub struct BootstrapRequest {
    pub data_plane_id: DataPlaneId,
    pub organisation_id: OrganisationId,
    pub region: Region,
    pub kubeconfig: SecretString,
}

#[cfg_attr(test, mockall::automock)]
pub trait ClusterBootstrapper: Send + Sync {
    fn bootstrap(
        &self,
        request: BootstrapRequest,
    ) -> impl Future<Output = Result<HeraldBinding, CoreError>> + Send;
}
