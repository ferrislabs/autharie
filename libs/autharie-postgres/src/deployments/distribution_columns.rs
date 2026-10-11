use autharie_domain::{
    CoreError,
    cells::{CellId, RealmName},
    dataplane::{
        cloud_provider::{
            ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeType,
        },
        cluster_profile::{ClusterMode, ClusterProfile, Replication},
        credential::CloudCredentialId,
    },
    deployments::distribution::Distribution,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct StoredProfile {
    mode: ClusterMode,
    control_plane_id: ControlPlaneOfferId,
    control_plane_kind: ControlPlaneKind,
    control_plane_monthly_price: u64,
    node_type: NodeType,
    min_nodes: u8,
    max_nodes: u8,
    replication: u8,
}

pub(super) struct DistributionColumns {
    pub distribution: &'static str,
    pub credential_id: Option<Uuid>,
    pub cluster_profile: Option<serde_json::Value>,
    pub cell_id: Option<Uuid>,
    pub realm: Option<String>,
    pub cell_slot_held: bool,
}

impl DistributionColumns {
    fn plain(distribution: &'static str) -> Self {
        Self {
            distribution,
            credential_id: None,
            cluster_profile: None,
            cell_id: None,
            realm: None,
            cell_slot_held: false,
        }
    }
}

pub(super) fn to_columns(distribution: &Distribution) -> Result<DistributionColumns, CoreError> {
    match distribution {
        Distribution::Shared => Ok(DistributionColumns::plain("shared")),
        Distribution::SelfHosted => Ok(DistributionColumns::plain("self_hosted")),
        Distribution::CustomerCloud {
            credential_id,
            profile,
        } => {
            let stored = StoredProfile {
                mode: profile.mode(),
                control_plane_id: profile.control_plane().id.clone(),
                control_plane_kind: profile.control_plane().kind,
                control_plane_monthly_price: profile.control_plane().monthly_price.minor_units(),
                node_type: profile.node_type().clone(),
                min_nodes: profile.min_nodes(),
                max_nodes: profile.max_nodes(),
                replication: profile.replication().get(),
            };
            let value = serde_json::to_value(stored).map_err(|e| {
                CoreError::InternalError(format!("a cluster profile cannot be stored: {e}"))
            })?;

            Ok(DistributionColumns {
                credential_id: Some(credential_id.0),
                cluster_profile: Some(value),
                ..DistributionColumns::plain("customer_cloud")
            })
        }
        Distribution::Pooled { cell_id, realm } => Ok(DistributionColumns {
            cell_id: Some(cell_id.0),
            realm: Some(realm.to_string()),
            cell_slot_held: true,
            ..DistributionColumns::plain("pooled")
        }),
    }
}

pub(super) fn from_columns(
    distribution: &str,
    credential_id: Option<Uuid>,
    cluster_profile: Option<serde_json::Value>,
    cell_id: Option<Uuid>,
    realm: Option<String>,
    deployment: Uuid,
) -> Result<Distribution, CoreError> {
    let unusable = |reason: String| {
        CoreError::InternalError(format!(
            "deployment {deployment} has an unusable distribution: {reason}"
        ))
    };

    match (distribution, credential_id, cluster_profile, cell_id, realm) {
        ("shared", None, None, None, None) => Ok(Distribution::Shared),
        ("self_hosted", None, None, None, None) => Ok(Distribution::SelfHosted),
        ("customer_cloud", Some(credential), Some(profile), None, None) => {
            let stored: StoredProfile =
                serde_json::from_value(profile).map_err(|e| unusable(e.to_string()))?;
            let replication = Replication::new(stored.replication)
                .ok_or_else(|| unusable("zero replicas".to_string()))?;
            let profile = ClusterProfile::restore(
                stored.mode,
                ControlPlaneOffer {
                    id: stored.control_plane_id,
                    kind: stored.control_plane_kind,
                    monthly_price: Money::new(stored.control_plane_monthly_price),
                },
                stored.node_type,
                stored.min_nodes,
                stored.max_nodes,
                replication,
            )
            .map_err(|e| unusable(e.to_string()))?;

            Ok(Distribution::CustomerCloud {
                credential_id: CloudCredentialId(credential),
                profile,
            })
        }
        ("pooled", None, None, Some(cell), Some(realm)) => {
            let realm = RealmName::try_from(realm.as_str()).map_err(|e| unusable(e.to_string()))?;

            Ok(Distribution::Pooled {
                cell_id: CellId(cell),
                realm,
            })
        }
        (other, ..) => Err(unusable(format!(
            "'{other}' with columns that do not belong to it"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customer_cloud(mode: ClusterMode, min: u8, max: u8, replicas: u8) -> Distribution {
        let profile = ClusterProfile::restore(
            mode,
            ControlPlaneOffer {
                id: ControlPlaneOfferId::new("kapsule-dedicated-4"),
                kind: ControlPlaneKind::Dedicated,
                monthly_price: Money::new(7_000),
            },
            NodeType::new("PRO2-S"),
            min,
            max,
            Replication::new(replicas).expect("replicas"),
        );

        Distribution::CustomerCloud {
            credential_id: CloudCredentialId(Uuid::new_v4()),
            profile: profile.expect("a valid profile"),
        }
    }

    fn round_trip(distribution: &Distribution) -> Distribution {
        let columns = to_columns(distribution).expect("stored");

        from_columns(
            columns.distribution,
            columns.credential_id,
            columns.cluster_profile,
            columns.cell_id,
            columns.realm,
            Uuid::nil(),
        )
        .expect("read back")
    }

    #[test]
    fn shared_and_self_hosted_round_trip_with_no_extra_columns() {
        for distribution in [Distribution::Shared, Distribution::SelfHosted] {
            let columns = to_columns(&distribution).expect("stored");

            assert!(columns.credential_id.is_none());
            assert!(columns.cluster_profile.is_none());
            assert_eq!(round_trip(&distribution), distribution);
        }
    }

    #[test]
    fn a_customer_cloud_distribution_round_trips_with_its_price_snapshot() {
        let distribution = customer_cloud(ClusterMode::Ha, 3, 10, 2);

        assert_eq!(round_trip(&distribution), distribution);
    }

    #[test]
    fn a_stored_profile_the_mode_no_longer_allows_is_refused_on_read() {
        let columns = to_columns(&customer_cloud(ClusterMode::Ha, 3, 10, 2)).expect("stored");
        let mut profile = columns.cluster_profile.expect("a profile");
        profile["min_nodes"] = serde_json::json!(1);

        let result = from_columns(
            columns.distribution,
            columns.credential_id,
            Some(profile),
            None,
            None,
            Uuid::nil(),
        );

        assert!(matches!(result, Err(CoreError::InternalError(_))));
    }

    #[test]
    fn a_customer_cloud_row_without_its_profile_is_refused_on_read() {
        let result = from_columns(
            "customer_cloud",
            Some(Uuid::new_v4()),
            None,
            None,
            None,
            Uuid::nil(),
        );

        assert!(matches!(result, Err(CoreError::InternalError(_))));
    }

    #[test]
    fn an_unknown_distribution_is_refused_on_read() {
        let result = from_columns("on_the_moon", None, None, None, None, Uuid::nil());

        assert!(matches!(result, Err(CoreError::InternalError(_))));
    }

    fn pooled() -> Distribution {
        Distribution::Pooled {
            cell_id: CellId(Uuid::new_v4()),
            realm: RealmName::try_from("acme").expect("a valid realm"),
        }
    }

    #[test]
    fn a_pooled_distribution_round_trips_and_holds_a_slot() {
        let distribution = pooled();
        let columns = to_columns(&distribution).expect("stored");

        assert_eq!(columns.distribution, "pooled");
        assert!(columns.cell_id.is_some());
        assert_eq!(columns.realm.as_deref(), Some("acme"));
        assert!(columns.cell_slot_held);
        assert!(columns.credential_id.is_none());
        assert!(columns.cluster_profile.is_none());
        assert_eq!(round_trip(&distribution), distribution);
    }

    #[test]
    fn other_distributions_hold_no_slot() {
        for distribution in [Distribution::Shared, Distribution::SelfHosted] {
            let columns = to_columns(&distribution).expect("stored");

            assert!(!columns.cell_slot_held);
            assert!(columns.cell_id.is_none());
            assert!(columns.realm.is_none());
        }
    }

    #[test]
    fn a_row_whose_columns_do_not_describe_a_pooled_deployment_is_refused_on_read() {
        let cell = Some(Uuid::new_v4());
        let realm = Some("acme".to_string());

        for (distribution, cell_id, realm) in [
            ("pooled", None, realm.clone()),
            ("pooled", cell, None),
            ("pooled", cell, Some("master".to_string())),
            ("shared", cell, realm.clone()),
            ("self_hosted", None, realm),
        ] {
            let result = from_columns(distribution, None, None, cell_id, realm, Uuid::nil());

            assert!(
                matches!(result, Err(CoreError::InternalError(_))),
                "{distribution}"
            );
        }
    }
}
