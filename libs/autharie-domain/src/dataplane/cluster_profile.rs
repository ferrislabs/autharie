use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::dataplane::cloud_provider::{
    ControlPlaneKind, ControlPlaneOffer, Money, NodeType, ProviderOffers,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ClusterMode {
    Dev,
    Standard,
    Ha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneAccess {
    MutualizedOnly,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeLimits {
    pub min_nodes: u8,
    pub max_nodes: Option<u8>,
    pub replicas: u8,
    pub database_instances: u8,
    pub autoscaling: bool,
    pub control_plane: ControlPlaneAccess,
}

impl ClusterMode {
    pub fn limits(&self) -> ModeLimits {
        match self {
            Self::Dev => ModeLimits {
                min_nodes: 1,
                max_nodes: Some(1),
                replicas: 1,
                database_instances: 1,
                autoscaling: false,
                control_plane: ControlPlaneAccess::MutualizedOnly,
            },
            Self::Standard => ModeLimits {
                min_nodes: 2,
                max_nodes: None,
                replicas: 2,
                database_instances: 1,
                autoscaling: true,
                control_plane: ControlPlaneAccess::Any,
            },
            Self::Ha => ModeLimits {
                min_nodes: 3,
                max_nodes: None,
                replicas: 2,
                database_instances: 3,
                autoscaling: true,
                control_plane: ControlPlaneAccess::Any,
            },
        }
    }
}

impl fmt::Display for ClusterMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dev => "dev",
            Self::Standard => "standard",
            Self::Ha => "ha",
        })
    }
}

/// How many replicas of FerrisKey run. At least one: a deployment with none
/// serves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
pub struct Replication(u8);

impl Replication {
    pub fn new(replicas: u8) -> Option<Self> {
        (replicas > 0).then_some(Self(replicas))
    }

    pub fn get(&self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("the minimum of {min_nodes} nodes is above the maximum of {max_nodes}")]
    NodeRangeInvalid { min_nodes: u8, max_nodes: u8 },

    #[error("{mode} needs at least {floor} nodes, not {min_nodes}")]
    BelowModeFloor {
        mode: ClusterMode,
        floor: u8,
        min_nodes: u8,
    },

    #[error("{mode} allows at most {ceiling} nodes, not {max_nodes}")]
    AboveModeCeiling {
        mode: ClusterMode,
        ceiling: u8,
        max_nodes: u8,
    },

    #[error("{replication} replicas cannot run on a minimum of {min_nodes} nodes")]
    ReplicationExceedsNodes { replication: u8, min_nodes: u8 },

    #[error("node type '{node_type}' is not offered in this region")]
    NodeTypeUnavailable { node_type: String },

    #[error("control plane '{id}' is not offered in this region")]
    ControlPlaneUnavailable { id: String },

    #[error("{mode} does not accept the dedicated control plane '{id}'")]
    ControlPlaneNotAllowedForMode { mode: ClusterMode, id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResizeError {
    #[error("{in_use} replicas are running and {target} allows {allowed}")]
    ReplicasExceedTarget {
        target: ClusterMode,
        in_use: u8,
        allowed: u8,
    },

    #[error(transparent)]
    Profile(#[from] ProfileError),
}

/// A mode and the adjustments made inside its limits.
///
/// Fields are private and `new` is the only way in, so a value of this type
/// has already been checked against its mode and the region's catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ClusterProfile {
    mode: ClusterMode,
    control_plane: ControlPlaneOffer,
    node_type: NodeType,
    min_nodes: u8,
    max_nodes: u8,
    replication: Replication,
}

impl ClusterProfile {
    pub fn new(
        mode: ClusterMode,
        control_plane: ControlPlaneOffer,
        node_type: NodeType,
        min_nodes: u8,
        max_nodes: u8,
        replication: Replication,
        catalog: &ProviderOffers,
    ) -> Result<Self, ProfileError> {
        let limits = mode.limits();

        if min_nodes > max_nodes {
            return Err(ProfileError::NodeRangeInvalid {
                min_nodes,
                max_nodes,
            });
        }

        if min_nodes < limits.min_nodes {
            return Err(ProfileError::BelowModeFloor {
                mode,
                floor: limits.min_nodes,
                min_nodes,
            });
        }

        if let Some(ceiling) = limits.max_nodes
            && max_nodes > ceiling
        {
            return Err(ProfileError::AboveModeCeiling {
                mode,
                ceiling,
                max_nodes,
            });
        }

        if replication.get() > min_nodes {
            return Err(ProfileError::ReplicationExceedsNodes {
                replication: replication.get(),
                min_nodes,
            });
        }

        if catalog.node_offer(&node_type).is_none() {
            return Err(ProfileError::NodeTypeUnavailable {
                node_type: node_type.as_str().to_string(),
            });
        }

        let offered = catalog.control_plane(&control_plane.id).ok_or_else(|| {
            ProfileError::ControlPlaneUnavailable {
                id: control_plane.id.as_str().to_string(),
            }
        })?;

        if limits.control_plane == ControlPlaneAccess::MutualizedOnly
            && offered.kind != ControlPlaneKind::Mutualized
        {
            return Err(ProfileError::ControlPlaneNotAllowedForMode {
                mode,
                id: offered.id.as_str().to_string(),
            });
        }

        Ok(Self {
            mode,
            control_plane: offered.clone(),
            node_type,
            min_nodes,
            max_nodes,
            replication,
        })
    }

    pub fn mode(&self) -> ClusterMode {
        self.mode
    }

    pub fn control_plane(&self) -> &ControlPlaneOffer {
        &self.control_plane
    }

    pub fn node_type(&self) -> &NodeType {
        &self.node_type
    }

    pub fn min_nodes(&self) -> u8 {
        self.min_nodes
    }

    pub fn max_nodes(&self) -> u8 {
        self.max_nodes
    }

    pub fn replication(&self) -> Replication {
        self.replication
    }

    /// The same cluster in another mode, keeping the node type and the
    /// control plane and moving the node range and replicas into the target's
    /// limits.
    ///
    /// Refused when the replicas running now exceed what the target allows:
    /// shrinking under them would take down capacity that is serving.
    pub fn resized_to(
        &self,
        target: ClusterMode,
        replicas_in_use: u8,
        catalog: &ProviderOffers,
    ) -> Result<Self, ResizeError> {
        let limits = target.limits();

        if replicas_in_use > limits.replicas {
            return Err(ResizeError::ReplicasExceedTarget {
                target,
                in_use: replicas_in_use,
                allowed: limits.replicas,
            });
        }

        let ceiling = limits.max_nodes.unwrap_or(u8::MAX);
        let min_nodes = self.min_nodes.max(limits.min_nodes).min(ceiling);
        let max_nodes = self.max_nodes.max(min_nodes).min(ceiling);
        let replication =
            Replication::new(limits.replicas.min(min_nodes).max(1)).unwrap_or(self.replication);

        Ok(Self::new(
            target,
            self.control_plane.clone(),
            self.node_type.clone(),
            min_nodes,
            max_nodes,
            replication,
            catalog,
        )?)
    }
}

/// What a profile costs per month: the nodes at the bottom of the range and
/// at the top, each plus the control plane. Egress is not included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct CostEstimate {
    pub min: Money,
    pub max: Money,
}

/// Priced from the catalog, not from the offers the profile was built with:
/// a profile kept for a month should be estimated at today's prices.
pub fn estimate_cluster_cost(
    profile: &ClusterProfile,
    catalog: &ProviderOffers,
) -> Result<CostEstimate, ProfileError> {
    let node = catalog.node_offer(&profile.node_type).ok_or_else(|| {
        ProfileError::NodeTypeUnavailable {
            node_type: profile.node_type.as_str().to_string(),
        }
    })?;
    let control_plane = catalog
        .control_plane(&profile.control_plane.id)
        .ok_or_else(|| ProfileError::ControlPlaneUnavailable {
            id: profile.control_plane.id.as_str().to_string(),
        })?;

    let fixed = control_plane.monthly_price;

    Ok(CostEstimate {
        min: node.monthly_price.times(profile.min_nodes).plus(fixed),
        max: node.monthly_price.times(profile.max_nodes).plus(fixed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataplane::cloud_provider::{ControlPlaneOfferId, NodeOffer};

    fn catalog() -> ProviderOffers {
        ProviderOffers {
            control_planes: vec![
                ControlPlaneOffer {
                    id: ControlPlaneOfferId::new("mutualized"),
                    kind: ControlPlaneKind::Mutualized,
                    monthly_price: Money::new(0),
                },
                ControlPlaneOffer {
                    id: ControlPlaneOfferId::new("dedicated-4"),
                    kind: ControlPlaneKind::Dedicated,
                    monthly_price: Money::new(7_000),
                },
            ],
            node_types: vec![NodeOffer {
                node_type: NodeType::new("small"),
                monthly_price: Money::new(1_000),
            }],
        }
    }

    fn offer(id: &str, kind: ControlPlaneKind) -> ControlPlaneOffer {
        ControlPlaneOffer {
            id: ControlPlaneOfferId::new(id),
            kind,
            monthly_price: Money::ZERO,
        }
    }

    fn mutualized() -> ControlPlaneOffer {
        offer("mutualized", ControlPlaneKind::Mutualized)
    }

    fn build(
        mode: ClusterMode,
        control_plane: ControlPlaneOffer,
        node_type: &str,
        min: u8,
        max: u8,
        replicas: u8,
    ) -> Result<ClusterProfile, ProfileError> {
        ClusterProfile::new(
            mode,
            control_plane,
            NodeType::new(node_type),
            min,
            max,
            Replication::new(replicas).expect("replicas"),
            &catalog(),
        )
    }

    /// @spec-ccp-5
    #[test]
    fn a_dev_profile_with_two_nodes_is_refused() {
        let result = build(ClusterMode::Dev, mutualized(), "small", 2, 2, 1);

        assert!(matches!(result, Err(ProfileError::AboveModeCeiling { .. })));
    }

    #[test]
    fn a_dev_profile_with_one_node_is_accepted() {
        assert!(build(ClusterMode::Dev, mutualized(), "small", 1, 1, 1).is_ok());
    }

    /// @spec-ccp-6
    #[test]
    fn a_ha_profile_with_two_minimum_nodes_is_refused() {
        let result = build(ClusterMode::Ha, mutualized(), "small", 2, 5, 2);

        assert_eq!(
            result,
            Err(ProfileError::BelowModeFloor {
                mode: ClusterMode::Ha,
                floor: 3,
                min_nodes: 2,
            })
        );
    }

    #[test]
    fn a_standard_profile_floors_at_two_nodes() {
        assert!(build(ClusterMode::Standard, mutualized(), "small", 2, 4, 2).is_ok());
        assert!(matches!(
            build(ClusterMode::Standard, mutualized(), "small", 1, 4, 1),
            Err(ProfileError::BelowModeFloor { floor: 2, .. })
        ));
    }

    #[test]
    fn the_minimum_cannot_exceed_the_maximum() {
        let result = build(ClusterMode::Standard, mutualized(), "small", 4, 3, 2);

        assert_eq!(
            result,
            Err(ProfileError::NodeRangeInvalid {
                min_nodes: 4,
                max_nodes: 3,
            })
        );
    }

    #[test]
    fn replication_never_exceeds_the_minimum_nodes() {
        let result = build(ClusterMode::Standard, mutualized(), "small", 2, 6, 3);

        assert_eq!(
            result,
            Err(ProfileError::ReplicationExceedsNodes {
                replication: 3,
                min_nodes: 2,
            })
        );
    }

    #[test]
    fn zero_replicas_do_not_exist() {
        assert!(Replication::new(0).is_none());
    }

    /// @spec-ccp-7
    #[test]
    fn a_node_type_absent_from_the_catalog_is_refused() {
        let result = build(ClusterMode::Standard, mutualized(), "gigantic", 2, 4, 2);

        assert_eq!(
            result,
            Err(ProfileError::NodeTypeUnavailable {
                node_type: "gigantic".to_string(),
            })
        );
    }

    /// @spec-ccp-17
    #[test]
    fn a_control_plane_absent_from_the_catalog_is_refused() {
        let missing = offer("dedicated-64", ControlPlaneKind::Dedicated);

        let result = build(ClusterMode::Standard, missing, "small", 2, 4, 2);

        assert_eq!(
            result,
            Err(ProfileError::ControlPlaneUnavailable {
                id: "dedicated-64".to_string(),
            })
        );
    }

    /// @spec-ccp-18
    #[test]
    fn a_dev_profile_with_a_dedicated_control_plane_is_refused() {
        let dedicated = offer("dedicated-4", ControlPlaneKind::Dedicated);

        let result = build(ClusterMode::Dev, dedicated, "small", 1, 1, 1);

        assert!(matches!(
            result,
            Err(ProfileError::ControlPlaneNotAllowedForMode {
                mode: ClusterMode::Dev,
                ..
            })
        ));
    }

    /// The catalog decides what an offer is, not the caller: a dedicated
    /// offer relabelled as mutualized must not slip into `dev`.
    #[test]
    fn the_catalog_decides_whether_a_control_plane_is_dedicated() {
        let relabelled = offer("dedicated-4", ControlPlaneKind::Mutualized);

        let result = build(ClusterMode::Dev, relabelled, "small", 1, 1, 1);

        assert!(matches!(
            result,
            Err(ProfileError::ControlPlaneNotAllowedForMode { .. })
        ));
    }

    /// @spec-ccp-19
    #[test]
    fn a_ha_profile_of_three_to_ten_catalog_nodes_is_accepted() {
        let dedicated = offer("dedicated-4", ControlPlaneKind::Dedicated);

        let profile = build(ClusterMode::Ha, dedicated, "small", 3, 10, 2).expect("valid");

        assert_eq!(profile.min_nodes(), 3);
        assert_eq!(profile.max_nodes(), 10);
    }

    /// @spec-ccp-8
    #[test]
    fn the_estimate_states_its_minimum_and_maximum() {
        let dedicated = offer("dedicated-4", ControlPlaneKind::Dedicated);
        let profile = build(ClusterMode::Ha, dedicated, "small", 3, 10, 2).expect("valid");

        let estimate = estimate_cluster_cost(&profile, &catalog()).expect("priced");

        assert_eq!(estimate.min, Money::new(3 * 1_000 + 7_000));
        assert_eq!(estimate.max, Money::new(10 * 1_000 + 7_000));
    }

    #[test]
    fn an_estimate_for_an_offer_the_catalog_dropped_is_refused() {
        let profile = build(ClusterMode::Standard, mutualized(), "small", 2, 4, 2).expect("valid");
        let emptied = ProviderOffers::default();

        assert!(matches!(
            estimate_cluster_cost(&profile, &emptied),
            Err(ProfileError::NodeTypeUnavailable { .. })
        ));
    }

    #[test]
    fn money_saturates_rather_than_wrapping() {
        assert_eq!(
            Money::new(u64::MAX).plus(Money::new(1)),
            Money::new(u64::MAX)
        );
        assert_eq!(Money::new(u64::MAX).times(2), Money::new(u64::MAX));
    }

    /// @spec-ccp-15
    #[test]
    fn resizing_dev_to_standard_adds_a_node_and_a_replica() {
        let dev = build(ClusterMode::Dev, mutualized(), "small", 1, 1, 1).expect("valid");

        let standard = dev
            .resized_to(ClusterMode::Standard, 1, &catalog())
            .expect("resized");

        assert_eq!(standard.mode(), ClusterMode::Standard);
        assert_eq!(standard.min_nodes(), 2);
        assert_eq!(standard.replication().get(), 2);
        assert_eq!(standard.node_type(), dev.node_type());
    }

    /// @spec-ccp-16
    #[test]
    fn resizing_ha_to_dev_is_refused_while_replicas_exceed_the_target() {
        let ha = build(ClusterMode::Ha, mutualized(), "small", 3, 5, 2).expect("valid");

        let result = ha.resized_to(ClusterMode::Dev, 2, &catalog());

        assert_eq!(
            result,
            Err(ResizeError::ReplicasExceedTarget {
                target: ClusterMode::Dev,
                in_use: 2,
                allowed: 1,
            })
        );
    }

    #[test]
    fn resizing_down_is_allowed_once_the_replicas_fit() {
        let ha = build(ClusterMode::Ha, mutualized(), "small", 3, 5, 2).expect("valid");

        let dev = ha
            .resized_to(ClusterMode::Dev, 1, &catalog())
            .expect("resized");

        assert_eq!((dev.min_nodes(), dev.max_nodes()), (1, 1));
        assert_eq!(dev.replication().get(), 1);
    }

    #[test]
    fn resizing_to_dev_refuses_a_dedicated_control_plane() {
        let dedicated = offer("dedicated-4", ControlPlaneKind::Dedicated);
        let ha = build(ClusterMode::Ha, dedicated, "small", 3, 5, 2).expect("valid");

        let result = ha.resized_to(ClusterMode::Dev, 1, &catalog());

        assert!(matches!(
            result,
            Err(ResizeError::Profile(
                ProfileError::ControlPlaneNotAllowedForMode { .. }
            ))
        ));
    }
}
