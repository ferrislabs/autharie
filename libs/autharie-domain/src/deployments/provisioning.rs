use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    dataplane::{entities::DataPlane, value_objects::DataPlaneStatus},
    deployments::DeploymentStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningStatus {
    Provisioning,
    Ready,
    Failed,
}

/// Where a customer deployment is on its way from "asked for" to "running".
///
/// Read from what already exists rather than stored: a plane without a Herald
/// identity is still being built, one with it is being installed, and one that
/// answers while its deployment has not come up is setting the IAM up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningStep {
    CreatingInfrastructure,
    InstallingDataPlane,
    SettingUpIam,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Provisioning {
    pub status: ProvisioningStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<ProvisioningStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl Provisioning {
    pub fn of(plane: &DataPlane, deployment: &DeploymentStatus) -> Option<Self> {
        match plane.status {
            DataPlaneStatus::Provisioning => Some(Self {
                status: ProvisioningStatus::Provisioning,
                step: Some(if plane.herald.is_some() {
                    ProvisioningStep::InstallingDataPlane
                } else {
                    ProvisioningStep::CreatingInfrastructure
                }),
                failure_reason: None,
            }),
            DataPlaneStatus::Active | DataPlaneStatus::Draining if iam_not_up_yet(deployment) => {
                Some(Self {
                    status: ProvisioningStatus::Provisioning,
                    step: Some(ProvisioningStep::SettingUpIam),
                    failure_reason: None,
                })
            }
            DataPlaneStatus::Active | DataPlaneStatus::Draining => Some(Self {
                status: ProvisioningStatus::Ready,
                step: None,
                failure_reason: None,
            }),
            DataPlaneStatus::Failed => Some(Self {
                status: ProvisioningStatus::Failed,
                step: None,
                failure_reason: plane.failure_reason.clone(),
            }),
            DataPlaneStatus::Disabled => None,
        }
    }
}

fn iam_not_up_yet(deployment: &DeploymentStatus) -> bool {
    matches!(
        deployment,
        DeploymentStatus::Pending | DeploymentStatus::Scheduling | DeploymentStatus::InProgress
    )
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        dataplane::herald_identity::HeraldBinding,
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
        let seen = Provisioning::of(&plane(), &DeploymentStatus::Pending).expect("shown");

        assert_eq!(seen.status, ProvisioningStatus::Provisioning);
        assert_eq!(seen.failure_reason, None);
    }

    #[test]
    fn a_failed_plane_carries_its_readable_reason() {
        let mut plane = plane();
        plane.fail("the provider quota in this account does not allow this cluster");

        let seen = Provisioning::of(&plane, &DeploymentStatus::Pending).expect("shown");

        assert_eq!(seen.status, ProvisioningStatus::Failed);
        assert_eq!(
            seen.failure_reason.as_deref(),
            Some("the provider quota in this account does not allow this cluster")
        );
    }

    #[test]
    fn an_active_plane_whose_deployment_is_up_is_ready_and_a_disabled_one_says_nothing() {
        let mut plane = plane();
        plane.status = DataPlaneStatus::Active;
        assert_eq!(
            Provisioning::of(&plane, &DeploymentStatus::Successful).map(|seen| seen.status),
            Some(ProvisioningStatus::Ready)
        );

        plane.status = DataPlaneStatus::Disabled;
        assert_eq!(Provisioning::of(&plane, &DeploymentStatus::Pending), None);
    }

    #[test]
    fn a_plane_without_a_herald_identity_is_creating_infrastructure() {
        let seen = Provisioning::of(&plane(), &DeploymentStatus::Pending).expect("shown");

        assert_eq!(seen.step, Some(ProvisioningStep::CreatingInfrastructure));
    }

    #[test]
    fn a_plane_with_a_herald_identity_is_installing_the_data_plane() {
        let mut plane = plane();
        plane.herald = Some(HeraldBinding {
            client_id: "herald-1".to_string(),
            subject: "subject-1".to_string(),
        });

        let seen = Provisioning::of(&plane, &DeploymentStatus::Pending).expect("shown");

        assert_eq!(seen.status, ProvisioningStatus::Provisioning);
        assert_eq!(seen.step, Some(ProvisioningStep::InstallingDataPlane));
    }

    #[test]
    fn an_active_plane_is_setting_the_iam_up_until_its_deployment_is_up() {
        let mut plane = plane();
        plane.status = DataPlaneStatus::Active;

        for status in [
            DeploymentStatus::Pending,
            DeploymentStatus::Scheduling,
            DeploymentStatus::InProgress,
        ] {
            let seen = Provisioning::of(&plane, &status).expect("shown");
            assert_eq!(seen.status, ProvisioningStatus::Provisioning);
            assert_eq!(seen.step, Some(ProvisioningStep::SettingUpIam));
        }

        let seen = Provisioning::of(&plane, &DeploymentStatus::Successful).expect("shown");
        assert_eq!(seen.status, ProvisioningStatus::Ready);
        assert_eq!(seen.step, None);
    }

    #[test]
    fn nothing_but_a_status_and_a_reason_is_serialised() {
        let building = serde_json::to_value(Provisioning::of(&plane(), &DeploymentStatus::Pending))
            .expect("json");
        let mut failed = plane();
        failed.fail("quota");

        let json = serde_json::to_value(Provisioning::of(&failed, &DeploymentStatus::Pending))
            .expect("json");

        assert_eq!(
            json,
            serde_json::json!({"status": "failed", "failure_reason": "quota"})
        );
        assert_eq!(
            building,
            serde_json::json!({"status": "provisioning", "step": "creating_infrastructure"})
        );
    }
}
