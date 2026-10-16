use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    dataplane::value_objects::{DataPlaneId, DeploymentResources},
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        environment::Environment, network::NetworkAccess, topology::Topology,
    },
    organisation::OrganisationId,
    user::UserId,
    version::Version,
};

pub fn instance(id: DeploymentId, status: DeploymentStatus, topology: Topology) -> Deployment {
    let at = DateTime::<Utc>::UNIX_EPOCH;

    Deployment {
        id,
        organisation_id: OrganisationId(Uuid::new_v4()),
        dataplane_id: DataPlaneId(Uuid::new_v4()),
        name: DeploymentName("cell-instance".to_string()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(1, 0, 0),
        status,
        environment: Environment::Production,
        namespace: "cells-cell-instance".to_string(),
        offer: None,
        restored_from: None,
        resources: DeploymentResources::DEFAULT,
        created_by: UserId(Uuid::new_v4()),
        created_at: at,
        updated_at: at,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: Default::default(),
        maintenance_window: None,
        network_access: NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
        topology,
    }
}
