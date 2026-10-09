use serde::Serialize;
use utoipa::ToSchema;

use crate::cells::{CellId, RealmName};
use crate::dataplane::{cluster_profile::ClusterProfile, credential::CloudCredentialId};
use crate::deployments::DeploymentKind;

/// How a deployment is hosted.
///
/// `CustomerCloud` carries its credential and its profile, so a deployment
/// that is in the customer's cloud cannot exist without saying whose account
/// and how big.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    #[default]
    Shared,
    SelfHosted,
    CustomerCloud {
        credential_id: CloudCredentialId,
        profile: ClusterProfile,
    },
    Pooled {
        cell_id: CellId,
        realm: RealmName,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DistributionError {
    #[error("a {kind} deployment cannot run in the customer's cloud")]
    NotAllowedForKind { kind: DeploymentKind },

    #[error("a {kind} deployment cannot be hosted as a realm in a cell")]
    NotPoolable { kind: DeploymentKind },
}

impl Distribution {
    /// Only FerrisKey is built to run on a cluster of its own or as a realm in a cell.
    pub fn allowed_for(&self, kind: &DeploymentKind) -> bool {
        match self {
            Self::CustomerCloud { .. } | Self::Pooled { .. } => *kind == DeploymentKind::Ferriskey,
            Self::Shared | Self::SelfHosted => true,
        }
    }

    pub fn checked_for(self, kind: &DeploymentKind) -> Result<Self, DistributionError> {
        if self.allowed_for(kind) {
            return Ok(self);
        }

        let kind = kind.clone();
        Err(match self {
            Self::Pooled { .. } => DistributionError::NotPoolable { kind },
            Self::Shared | Self::SelfHosted | Self::CustomerCloud { .. } => {
                DistributionError::NotAllowedForKind { kind }
            }
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::dataplane::{
        cloud_provider::{
            ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeOffer, NodeType,
            ProviderOffers,
        },
        cluster_profile::{ClusterMode, Replication},
    };

    pub(crate) fn customer_cloud() -> Distribution {
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

        Distribution::CustomerCloud {
            credential_id: CloudCredentialId(Uuid::new_v4()),
            profile,
        }
    }

    fn pooled() -> Distribution {
        Distribution::Pooled {
            cell_id: CellId(Uuid::nil()),
            realm: RealmName::try_from("acme").expect("valid realm"),
        }
    }

    /// @spec-fpr-13
    #[test]
    fn only_a_ferriskey_deployment_can_be_pooled() {
        assert!(pooled().allowed_for(&DeploymentKind::Ferriskey));
        assert!(!pooled().allowed_for(&DeploymentKind::Keycloak));
        assert!(pooled().checked_for(&DeploymentKind::Ferriskey).is_ok());
        assert_eq!(
            pooled().checked_for(&DeploymentKind::Keycloak),
            Err(DistributionError::NotPoolable {
                kind: DeploymentKind::Keycloak
            })
        );
    }

    #[test]
    fn each_refusal_names_the_distribution_it_is_about() {
        let customer = customer_cloud()
            .checked_for(&DeploymentKind::Keycloak)
            .expect_err("keycloak in the customer's cloud");
        let pooled = pooled()
            .checked_for(&DeploymentKind::Keycloak)
            .expect_err("keycloak in a cell");

        assert_eq!(
            customer.to_string(),
            "a keycloak deployment cannot run in the customer's cloud"
        );
        assert_eq!(
            pooled.to_string(),
            "a keycloak deployment cannot be hosted as a realm in a cell"
        );
    }

    #[test]
    fn a_pooled_distribution_serialises_in_snake_case() {
        let value = serde_json::to_value(pooled()).expect("serialises");

        assert_eq!(
            value,
            serde_json::json!({
                "pooled": {
                    "cell_id": "00000000-0000-0000-0000-000000000000",
                    "realm": "acme"
                }
            })
        );
    }

    /// @spec-ccp-9
    #[test]
    fn a_keycloak_deployment_cannot_use_customer_cloud() {
        let result = customer_cloud().checked_for(&DeploymentKind::Keycloak);

        assert_eq!(
            result,
            Err(DistributionError::NotAllowedForKind {
                kind: DeploymentKind::Keycloak
            })
        );
    }

    #[test]
    fn a_ferriskey_deployment_can_use_customer_cloud() {
        assert!(
            customer_cloud()
                .checked_for(&DeploymentKind::Ferriskey)
                .is_ok()
        );
    }

    #[test]
    fn every_kind_can_be_shared_or_self_hosted() {
        for kind in [DeploymentKind::Ferriskey, DeploymentKind::Keycloak] {
            assert!(Distribution::Shared.allowed_for(&kind));
            assert!(Distribution::SelfHosted.allowed_for(&kind));
        }
    }

    #[test]
    fn a_deployment_is_shared_unless_it_says_otherwise() {
        assert_eq!(Distribution::default(), Distribution::Shared);
    }
}
