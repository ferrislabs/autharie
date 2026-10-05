use serde::Serialize;
use utoipa::ToSchema;

use crate::dataplane::{entities::DataPlane, value_objects::DataPlaneStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningStatus {
    Provisioning,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Provisioning {
    pub status: ProvisioningStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl Provisioning {
    pub fn of(plane: &DataPlane) -> Option<Self> {
        match plane.status {
            DataPlaneStatus::Provisioning => Some(Self {
                status: ProvisioningStatus::Provisioning,
                failure_reason: None,
            }),
            DataPlaneStatus::Active | DataPlaneStatus::Draining => Some(Self {
                status: ProvisioningStatus::Ready,
                failure_reason: None,
            }),
            DataPlaneStatus::Failed => Some(Self {
                status: ProvisioningStatus::Failed,
                failure_reason: plane.failure_reason.clone(),
            }),
            DataPlaneStatus::Disabled => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        dataplane::value_objects::{Capacity, DataPlaneAllocation, Region},
        deployments::DeploymentId,
        organisation::OrganisationId,
    };

    fn plane() -> DataPlane {
        DataPlane::new(
            DataPlaneAllocation::Customer {
                organisation_id: OrganisationId(Uuid::new_v4()),
                deployment_id: DeploymentId(Uuid::new_v4()),
                credential_id: crate::dataplane::credential::CloudCredentialId(Uuid::new_v4()),
            },
            Region::new("fr-par"),
            Capacity::new(1, 1, 1).expect("capacity"),
        )
    }

    #[test]
    fn a_plane_being_built_is_provisioning() {
        let seen = Provisioning::of(&plane()).expect("shown");

        assert_eq!(seen.status, ProvisioningStatus::Provisioning);
        assert_eq!(seen.failure_reason, None);
    }

    #[test]
    fn a_failed_plane_carries_its_readable_reason() {
        let mut plane = plane();
        plane.fail("the provider quota in this account does not allow this cluster");

        let seen = Provisioning::of(&plane).expect("shown");

        assert_eq!(seen.status, ProvisioningStatus::Failed);
        assert_eq!(
            seen.failure_reason.as_deref(),
            Some("the provider quota in this account does not allow this cluster")
        );
    }

    #[test]
    fn an_active_plane_is_ready_and_a_disabled_one_says_nothing() {
        let mut plane = plane();
        plane.status = DataPlaneStatus::Active;
        assert_eq!(
            Provisioning::of(&plane).map(|seen| seen.status),
            Some(ProvisioningStatus::Ready)
        );

        plane.status = DataPlaneStatus::Disabled;
        assert_eq!(Provisioning::of(&plane), None);
    }

    #[test]
    fn nothing_but_a_status_and_a_reason_is_serialised() {
        let building = serde_json::to_value(Provisioning::of(&plane())).expect("json");
        let mut failed = plane();
        failed.fail("quota");

        let json = serde_json::to_value(Provisioning::of(&failed)).expect("json");

        assert_eq!(
            json,
            serde_json::json!({"status": "failed", "failure_reason": "quota"})
        );
        assert_eq!(building, serde_json::json!({"status": "provisioning"}));
    }
}
