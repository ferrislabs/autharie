use std::sync::{Arc, Mutex};

use autharie_domain::{
    CoreError,
    dataplane::{
        bootstrap::{BootstrapRequest, ClusterBootstrapper},
        cloud_provider::Provider,
        credential::{CloudCredentialId, CloudCredentialStore, CredentialError, SecretString},
        entities::DataPlane,
        herald_identity::HeraldBinding,
        inventory::{ClusterInventory, ProvisionedResource},
        ports::DataPlaneRepository,
        value_objects::{
            DataPlaneId, DataPlaneMode, DeploymentResources, PlacementRequest, Region,
        },
    },
    organisation::OrganisationId,
    version::Version,
};
use chrono::{DateTime, Utc};

#[derive(Clone, Default)]
pub(crate) struct FakeInventory {
    entries: Arc<Mutex<Vec<(DataPlaneId, ProvisionedResource, bool)>>>,
}

impl FakeInventory {
    pub(crate) fn seed(&self, id: DataPlaneId, resource: ProvisionedResource) {
        self.entries
            .lock()
            .expect("inventory lock")
            .push((id, resource, false));
    }

    pub(crate) fn all(&self) -> Vec<(ProvisionedResource, bool)> {
        self.entries
            .lock()
            .expect("inventory lock")
            .iter()
            .map(|(_, resource, released)| (resource.clone(), *released))
            .collect()
    }

    pub(crate) fn recorded(&self) -> usize {
        self.entries.lock().expect("inventory lock").len()
    }
}

impl ClusterInventory for FakeInventory {
    async fn record(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        self.entries.lock().expect("inventory lock").push((
            *data_plane_id,
            resource.clone(),
            false,
        ));
        Ok(())
    }

    async fn unreleased(
        &self,
        data_plane_id: &DataPlaneId,
    ) -> Result<Vec<ProvisionedResource>, CoreError> {
        Ok(self
            .entries
            .lock()
            .expect("inventory lock")
            .iter()
            .filter(|(id, _, released)| id == data_plane_id && !released)
            .map(|(_, resource, _)| resource.clone())
            .collect())
    }

    async fn mark_released(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        for (id, entry, released) in self.entries.lock().expect("inventory lock").iter_mut() {
            if id == data_plane_id && entry == resource {
                *released = true;
            }
        }
        Ok(())
    }
}

pub(crate) struct FakeStore {
    pub secret: String,
}

impl CloudCredentialStore for FakeStore {
    async fn put(
        &self,
        _organisation_id: OrganisationId,
        _provider: Provider,
        _secret: SecretString,
    ) -> Result<CloudCredentialId, CredentialError> {
        Err(CredentialError::Store("not supported".to_string()))
    }

    async fn get_for_provisioning(
        &self,
        _id: &CloudCredentialId,
    ) -> Result<SecretString, CredentialError> {
        Ok(SecretString::new(self.secret.clone()))
    }

    async fn delete(&self, _id: &CloudCredentialId) -> Result<(), CredentialError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakeRepository {
    pub data_plane: Option<DataPlane>,
}

impl DataPlaneRepository for FakeRepository {
    async fn find_by_herald_subject(&self, _subject: &str) -> Result<Option<DataPlane>, CoreError> {
        Ok(None)
    }

    async fn find_by_id(&self, id: &DataPlaneId) -> Result<Option<DataPlane>, CoreError> {
        Ok(self
            .data_plane
            .as_ref()
            .filter(|data_plane| &data_plane.id == id)
            .cloned())
    }

    async fn find_active_shared_by_region(
        &self,
        _region: &Region,
    ) -> Result<Vec<DataPlane>, CoreError> {
        Ok(Vec::new())
    }

    async fn find_available(
        &self,
        _request: PlacementRequest,
    ) -> Result<Option<DataPlane>, CoreError> {
        Ok(None)
    }

    async fn region_is_served(&self, _region: &Region) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn region_blocked_by_deployment_count(
        &self,
        _region: &Region,
        _mode: DataPlaneMode,
        _resources: DeploymentResources,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn find_dedicated_for_organisation(
        &self,
        _organisation_id: &OrganisationId,
        _region: &Region,
    ) -> Result<Option<DataPlane>, CoreError> {
        Ok(None)
    }

    async fn list_all(&self) -> Result<Vec<DataPlane>, CoreError> {
        Ok(Vec::new())
    }

    async fn current_load(&self, _id: &DataPlaneId) -> Result<u32, CoreError> {
        Ok(0)
    }

    async fn save(&self, _dataplane: &DataPlane) -> Result<(), CoreError> {
        Ok(())
    }

    async fn touch_last_seen(
        &self,
        _id: &DataPlaneId,
        _at: DateTime<Utc>,
        _operator_version: Option<Version>,
        _gateway_address: Option<String>,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }
}

pub(crate) struct Bootstrapped {
    pub data_plane_id: DataPlaneId,
    pub kubeconfig: String,
    pub recorded_when_called: usize,
}

#[derive(Clone)]
pub(crate) struct FakeBootstrapper {
    inventory: FakeInventory,
    calls: Arc<Mutex<Vec<Bootstrapped>>>,
}

impl FakeBootstrapper {
    pub(crate) fn new(inventory: FakeInventory) -> Self {
        Self {
            inventory,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(DataPlaneId, String, usize)> {
        self.calls
            .lock()
            .expect("bootstrapper lock")
            .iter()
            .map(|call| {
                (
                    call.data_plane_id,
                    call.kubeconfig.clone(),
                    call.recorded_when_called,
                )
            })
            .collect()
    }
}

impl ClusterBootstrapper for FakeBootstrapper {
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<HeraldBinding, CoreError> {
        let client_id = format!("herald-{}", request.data_plane_id);
        self.calls
            .lock()
            .expect("bootstrapper lock")
            .push(Bootstrapped {
                data_plane_id: request.data_plane_id,
                kubeconfig: request.kubeconfig.expose().to_string(),
                recorded_when_called: self.inventory.recorded(),
            });
        Ok(HeraldBinding {
            subject: format!("sa-{client_id}"),
            client_id,
        })
    }
}
