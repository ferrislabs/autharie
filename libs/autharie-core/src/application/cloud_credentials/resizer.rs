use std::{
    fmt,
    sync::{Arc, Mutex},
};

use autharie_domain::{
    CoreError,
    dataplane::{
        bootstrap::{BootstrapRequest, ClusterBootstrapper},
        cluster_profile::ClusterProfile,
        herald_identity::HeraldBinding,
        provisioner::{ClusterProvisioner, ProvisionRequest, ProvisionedCluster},
        value_objects::DataPlaneId,
    },
};
use autharie_scaleway::{ScalewayConfig, ScalewayError, ScalewayProvisioner};

use crate::infrastructure::pooled::{PooledCredentialStore, PooledDataPlanes, PooledInventory};

const ONLY_RESIZES: &str = "this process can resize a customer cluster but not build one";

pub struct ResizeOnly;

impl ClusterBootstrapper for ResizeOnly {
    async fn bootstrap(&self, _request: BootstrapRequest) -> Result<HeraldBinding, CoreError> {
        Err(CoreError::ProvisioningUnavailable {
            reason: ONLY_RESIZES.to_string(),
        })
    }
}

type ScalewayResizer =
    ScalewayProvisioner<PooledCredentialStore, PooledInventory, PooledDataPlanes, ResizeOnly>;

#[derive(Default)]
pub struct RecordingResizer {
    calls: Mutex<Vec<(DataPlaneId, ClusterProfile)>>,
    failing: bool,
}

impl RecordingResizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn failing() -> Self {
        Self {
            failing: true,
            ..Self::default()
        }
    }

    pub fn calls(&self) -> Vec<(DataPlaneId, ClusterProfile)> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub enum ClusterResizers {
    #[default]
    Unconfigured,
    Recording(Arc<RecordingResizer>),
    Scaleway(Arc<ScalewayResizer>),
}

impl fmt::Debug for ClusterResizers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unconfigured => formatter.write_str("Unconfigured"),
            Self::Recording(_) => formatter.write_str("Recording"),
            Self::Scaleway(_) => formatter.write_str("Scaleway"),
        }
    }
}

impl ClusterResizers {
    pub fn scaleway(
        config: ScalewayConfig,
        credentials: PooledCredentialStore,
        pool: sqlx::PgPool,
    ) -> Result<Self, ScalewayError> {
        Ok(Self::Scaleway(Arc::new(ScalewayProvisioner::new(
            config,
            credentials,
            PooledInventory::new(pool.clone()),
            PooledDataPlanes::new(pool),
            ResizeOnly,
        )?)))
    }

    pub fn is_configured(&self) -> bool {
        !matches!(self, Self::Unconfigured)
    }
}

fn unconfigured() -> CoreError {
    CoreError::ProvisioningUnavailable {
        reason: "resizing a customer cluster is not enabled on this installation".to_string(),
    }
}

impl ClusterProvisioner for ClusterResizers {
    async fn provision(&self, _request: ProvisionRequest) -> Result<ProvisionedCluster, CoreError> {
        Err(CoreError::ProvisioningUnavailable {
            reason: ONLY_RESIZES.to_string(),
        })
    }

    async fn deprovision(&self, _id: &DataPlaneId) -> Result<(), CoreError> {
        Err(CoreError::ProvisioningUnavailable {
            reason: ONLY_RESIZES.to_string(),
        })
    }

    async fn resize(&self, id: &DataPlaneId, profile: &ClusterProfile) -> Result<(), CoreError> {
        match self {
            Self::Unconfigured => Err(unconfigured()),
            Self::Recording(recording) => {
                if let Ok(mut calls) = recording.calls.lock() {
                    calls.push((*id, profile.clone()));
                }
                if recording.failing {
                    return Err(CoreError::InternalError("the provider is down".to_string()));
                }
                Ok(())
            }
            Self::Scaleway(provisioner) => provisioner.resize(id, profile).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use autharie_domain::dataplane::{
        cloud_provider::{
            ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeOffer, NodeType,
            ProviderOffers,
        },
        cluster_profile::{ClusterMode, Replication},
    };
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn an_installation_without_customer_cloud_refuses_to_resize_readably() {
        let control_plane = ControlPlaneOffer {
            id: ControlPlaneOfferId::new("mutualized"),
            kind: ControlPlaneKind::Mutualized,
            monthly_price: Money::ZERO,
        };
        let catalog = ProviderOffers {
            control_planes: vec![control_plane.clone()],
            node_types: vec![NodeOffer {
                node_type: NodeType::new("small"),
                monthly_price: Money::new(1_000),
            }],
        };
        let profile = ClusterProfile::new(
            ClusterMode::Dev,
            control_plane,
            NodeType::new("small"),
            1,
            1,
            Replication::new(1).expect("one replica"),
            &catalog,
        )
        .expect("valid profile");

        let result = ClusterResizers::Unconfigured
            .resize(&DataPlaneId(Uuid::new_v4()), &profile)
            .await;

        assert!(matches!(
            result,
            Err(CoreError::ProvisioningUnavailable { .. })
        ));
        assert!(!ClusterResizers::Unconfigured.is_configured());
    }
}
