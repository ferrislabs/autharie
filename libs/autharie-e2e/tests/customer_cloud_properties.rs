use autharie_domain::dataplane::{
    cloud_provider::{
        ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeOffer, NodeType,
        ProviderOffers,
    },
    cluster_profile::{
        ClusterMode, ClusterProfile, ProfileError, Replication, ResizeError, estimate_cluster_cost,
    },
};
use autharie_e2e::support::customer_cloud::{
    DEDICATED, DEDICATED_PRICE, MEDIUM, MEDIUM_PRICE, MUTUALIZED, MUTUALIZED_PRICE, SMALL,
    SMALL_PRICE, catalog, control_plane,
};
use proptest::prelude::*;

const MODES: [ClusterMode; 3] = [ClusterMode::Dev, ClusterMode::Standard, ClusterMode::Ha];
const UNKNOWN_CONTROL_PLANE: &str = "kapsule-dedicated-64";
const UNKNOWN_NODE: &str = "GIGANTIC-XL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plane {
    Mutualized,
    Dedicated,
    Absent,
}

const PLANES: [Plane; 3] = [Plane::Mutualized, Plane::Dedicated, Plane::Absent];

fn plane_offer(plane: Plane) -> ControlPlaneOffer {
    match plane {
        Plane::Mutualized => {
            control_plane(MUTUALIZED, ControlPlaneKind::Mutualized, MUTUALIZED_PRICE)
        }
        Plane::Dedicated => control_plane(DEDICATED, ControlPlaneKind::Dedicated, DEDICATED_PRICE),
        Plane::Absent => control_plane(UNKNOWN_CONTROL_PLANE, ControlPlaneKind::Mutualized, 0),
    }
}

fn floor(mode: ClusterMode) -> u8 {
    match mode {
        ClusterMode::Dev => 1,
        ClusterMode::Standard => 2,
        ClusterMode::Ha => 3,
    }
}

fn ceiling(mode: ClusterMode) -> Option<u8> {
    match mode {
        ClusterMode::Dev => Some(1),
        ClusterMode::Standard | ClusterMode::Ha => None,
    }
}

fn replicas_allowed(mode: ClusterMode) -> u8 {
    match mode {
        ClusterMode::Dev => 1,
        ClusterMode::Standard | ClusterMode::Ha => 2,
    }
}

fn spec_accepts(
    mode: ClusterMode,
    min: u8,
    max: u8,
    replicas: u8,
    plane: Plane,
    node_listed: bool,
) -> bool {
    let range = min <= max;
    let floor_met = min >= floor(mode);
    let ceiling_met = ceiling(mode).is_none_or(|ceiling| max <= ceiling);
    let replicated = replicas <= min;
    let control_plane_listed = plane != Plane::Absent;
    let dev_is_mutualized = !(mode == ClusterMode::Dev && plane == Plane::Dedicated);

    range
        && floor_met
        && ceiling_met
        && replicated
        && node_listed
        && control_plane_listed
        && dev_is_mutualized
}

fn profile_holds_the_spec(profile: &ClusterProfile) -> bool {
    let mode = profile.mode();
    let listed_plane = match profile.control_plane().id.as_str() {
        MUTUALIZED => Some(ControlPlaneKind::Mutualized),
        DEDICATED => Some(ControlPlaneKind::Dedicated),
        _ => None,
    };
    let node_listed = [SMALL, MEDIUM].contains(&profile.node_type().as_str());
    let plane = match listed_plane {
        Some(ControlPlaneKind::Mutualized) => Plane::Mutualized,
        Some(ControlPlaneKind::Dedicated) => Plane::Dedicated,
        None => Plane::Absent,
    };

    spec_accepts(
        mode,
        profile.min_nodes(),
        profile.max_nodes(),
        profile.replication().get(),
        plane,
        node_listed,
    ) && listed_plane == Some(profile.control_plane().kind)
}

fn price_of_node(node_type: &str) -> u64 {
    match node_type {
        SMALL => SMALL_PRICE,
        MEDIUM => MEDIUM_PRICE,
        other => panic!("{other} is not in the catalog"),
    }
}

fn price_of_plane(plane: &ControlPlaneOffer) -> u64 {
    match plane.id.as_str() {
        MUTUALIZED => MUTUALIZED_PRICE,
        DEDICATED => DEDICATED_PRICE,
        other => panic!("{other} is not in the catalog"),
    }
}

struct Case {
    mode: ClusterMode,
    min: u8,
    max: u8,
    replicas: u8,
    plane: Plane,
    node_type: &'static str,
}

fn cases() -> Vec<Case> {
    let mut all = Vec::new();
    for mode in MODES {
        for min in 0..=12u8 {
            for max in 0..=12u8 {
                for replicas in 1..=5u8 {
                    for plane in PLANES {
                        for node_type in [SMALL, MEDIUM, UNKNOWN_NODE] {
                            all.push(Case {
                                mode,
                                min,
                                max,
                                replicas,
                                plane,
                                node_type,
                            });
                        }
                    }
                }
            }
        }
    }
    all
}

fn build(case: &Case, offers: &ProviderOffers) -> Result<ClusterProfile, ProfileError> {
    ClusterProfile::new(
        case.mode,
        plane_offer(case.plane),
        NodeType::new(case.node_type),
        case.min,
        case.max,
        Replication::new(case.replicas).expect("at least one replica"),
        offers,
    )
}

fn accepted() -> Vec<ClusterProfile> {
    let offers = catalog();
    cases()
        .iter()
        .filter_map(|case| build(case, &offers).ok())
        .collect()
}

#[test]
fn a_profile_is_accepted_exactly_when_the_spec_rules_hold() {
    let offers = catalog();
    let all = cases();

    let mut accepted = 0;
    for case in &all {
        let expected = spec_accepts(
            case.mode,
            case.min,
            case.max,
            case.replicas,
            case.plane,
            case.node_type != UNKNOWN_NODE,
        );
        let outcome = build(case, &offers);

        assert_eq!(
            outcome.is_ok(),
            expected,
            "{:?} min {} max {} replicas {} control plane {:?} node {} gave {:?}",
            case.mode,
            case.min,
            case.max,
            case.replicas,
            case.plane,
            case.node_type,
            outcome
        );
        if let Ok(profile) = outcome {
            accepted += 1;
            assert!(profile_holds_the_spec(&profile));
            assert_eq!(profile.control_plane(), &catalog_offer(&offers, case.plane));
        }
    }

    assert_eq!(all.len(), 3 * 13 * 13 * 5 * 3 * 3);
    assert!(accepted > 0);
    assert!(accepted < all.len());
}

fn catalog_offer(offers: &ProviderOffers, plane: Plane) -> ControlPlaneOffer {
    offers
        .control_plane(&ControlPlaneOfferId::new(plane_offer(plane).id.as_str()))
        .cloned()
        .expect("a catalog offer")
}

#[test]
fn every_accepted_profile_is_estimated_within_its_range_at_catalog_prices() {
    let offers = catalog();
    let profiles = accepted();
    assert!(!profiles.is_empty());

    for profile in &profiles {
        let estimate = estimate_cluster_cost(profile, &offers).expect("an estimate");
        let node = price_of_node(profile.node_type().as_str());
        let fixed = price_of_plane(profile.control_plane());

        assert!(estimate.min <= estimate.max, "{profile:?}");
        assert_eq!(
            estimate.min.minor_units(),
            node * u64::from(profile.min_nodes()) + fixed
        );
        assert_eq!(
            estimate.max.minor_units(),
            node * u64::from(profile.max_nodes()) + fixed
        );
    }
}

#[test]
fn every_accepted_profile_survives_a_restore_unchanged() {
    for profile in accepted() {
        let restored = ClusterProfile::restore(
            profile.mode(),
            profile.control_plane().clone(),
            profile.node_type().clone(),
            profile.min_nodes(),
            profile.max_nodes(),
            profile.replication(),
        );

        assert_eq!(restored, Ok(profile));
    }
}

#[test]
fn a_resize_never_yields_an_invalid_profile_and_refuses_replicas_in_use_above_the_target() {
    let offers = catalog();
    let mut resized_ok = 0;
    let mut refused = 0;

    for profile in accepted() {
        for target in MODES {
            for in_use in 0..=4u8 {
                let outcome = profile.resized_to(target, in_use, &offers);
                let dedicated = profile.control_plane().kind == ControlPlaneKind::Dedicated;

                if in_use > replicas_allowed(target) {
                    assert_eq!(
                        outcome,
                        Err(ResizeError::ReplicasExceedTarget {
                            target,
                            in_use,
                            allowed: replicas_allowed(target),
                        }),
                        "{profile:?} to {target:?} with {in_use} in use"
                    );
                    refused += 1;
                } else if target == ClusterMode::Dev && dedicated {
                    assert!(
                        matches!(
                            outcome,
                            Err(ResizeError::Profile(
                                ProfileError::ControlPlaneNotAllowedForMode { .. }
                            ))
                        ),
                        "{profile:?} to dev gave {outcome:?}"
                    );
                    refused += 1;
                } else {
                    let resized = outcome.unwrap_or_else(|error| {
                        panic!("{profile:?} to {target:?} with {in_use} in use: {error}")
                    });
                    assert_eq!(resized.mode(), target);
                    assert!(profile_holds_the_spec(&resized), "{resized:?}");
                    assert_eq!(resized.node_type(), profile.node_type());
                    assert_eq!(resized.control_plane(), profile.control_plane());
                    resized_ok += 1;
                }
            }
        }
    }

    assert!(resized_ok > 0);
    assert!(refused > 0);
}

fn priced(node_price: u64, plane_price: u64) -> ProviderOffers {
    ProviderOffers {
        control_planes: vec![
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new(MUTUALIZED),
                kind: ControlPlaneKind::Mutualized,
                monthly_price: Money::new(plane_price),
            },
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new(DEDICATED),
                kind: ControlPlaneKind::Dedicated,
                monthly_price: Money::new(plane_price),
            },
        ],
        node_types: vec![NodeOffer {
            node_type: NodeType::new(SMALL),
            monthly_price: Money::new(node_price),
        }],
    }
}

fn clamped(exact: u128) -> u64 {
    u64::try_from(exact).unwrap_or(u64::MAX)
}

proptest! {
    #[test]
    fn an_estimate_is_ordered_and_never_wraps_whatever_the_prices(
        node_price in any::<u64>(),
        plane_price in any::<u64>(),
        min in 3..=10u8,
        spread in 0..=5u8,
        dedicated in any::<bool>(),
    ) {
        let offers = priced(node_price, plane_price);
        let max = min + spread;
        let plane = if dedicated {
            plane_offer(Plane::Dedicated)
        } else {
            plane_offer(Plane::Mutualized)
        };
        let profile = ClusterProfile::new(
            ClusterMode::Ha,
            plane,
            NodeType::new(SMALL),
            min,
            max,
            Replication::new(2).expect("replicas"),
            &offers,
        ).expect("an ha profile inside its limits");

        let estimate = estimate_cluster_cost(&profile, &offers).expect("an estimate");

        prop_assert!(estimate.min <= estimate.max);
        prop_assert_eq!(
            estimate.min.minor_units(),
            clamped(u128::from(node_price) * u128::from(min) + u128::from(plane_price))
        );
        prop_assert_eq!(
            estimate.max.minor_units(),
            clamped(u128::from(node_price) * u128::from(max) + u128::from(plane_price))
        );
    }
}
