//! How much of a thing a deployment may hold.
//!
//! A quota has two sources. The offer a deployment was bought on says what it
//! gets, in [`default_quota`]; platform staff can replace that for one
//! organisation, whatever its plan, and the replacement applies to each of the
//! organisation's deployments. The override is the more specific statement, so
//! it wins in both directions.
//!
//! The kinds are a closed set, the way [`crate::features::Feature`] is, and the
//! defaults a total function over them: a kind with no default for some offer
//! would be a quota that silently means "none" or "any".

pub mod ports;
pub mod service;

#[cfg(test)]
pub mod fixtures;

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{CoreError, offers::Offer};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum QuotaKind {
    /// Measured and shown, never enforced.
    Users,
    McpServers,
    Policies,
    Webhooks,
}

impl QuotaKind {
    pub const ALL: [Self; 4] = [
        Self::Users,
        Self::McpServers,
        Self::Policies,
        Self::Webhooks,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::McpServers => "mcp_servers",
            Self::Policies => "policies",
            Self::Webhooks => "webhooks",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Users => "Users of the deployment (measured, not enforced).",
            Self::McpServers => "MCP servers registered on a deployment.",
            Self::Policies => "Agent policies on a deployment.",
            Self::Webhooks => "Webhooks on a deployment.",
        }
    }

    /// Whether going over refuses anything.
    pub fn is_enforced(&self) -> bool {
        !matches!(self, Self::Users)
    }
}

impl fmt::Display for QuotaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for QuotaKind {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| CoreError::UnknownQuota {
                value: value.to_string(),
            })
    }
}

/// A limit, or the absence of one.
///
/// Tagged on the wire rather than a bare number with `null` for unlimited:
/// an optional quota (an override that may not exist) would then have two
/// meanings for `null`. `0` is a real quota: none may be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", content = "max", rename_all = "snake_case")]
pub enum Quota {
    Limited(u32),
    Unlimited,
}

impl Quota {
    pub fn allows(&self, held: usize) -> bool {
        match self {
            Self::Limited(max) => held < *max as usize,
            Self::Unlimited => true,
        }
    }
}

/// What each offer allows.
///
/// A deployment that has no offer (created before offers existed) is not
/// limited by default: its resources were never picked from a list, and
/// capping it would be a decision nobody made. An override still applies.
pub fn default_quota(offer: Option<Offer>, kind: QuotaKind) -> Quota {
    let Some(offer) = offer else {
        return Quota::Unlimited;
    };

    match kind {
        QuotaKind::Users => Quota::Unlimited,
        QuotaKind::McpServers => match offer {
            Offer::Free => Quota::Limited(1),
            Offer::Sandbox => Quota::Limited(3),
            Offer::Standard => Quota::Limited(10),
            Offer::Scale => Quota::Limited(50),
            Offer::Private => Quota::Unlimited,
        },
        QuotaKind::Policies => match offer {
            Offer::Free => Quota::Limited(5),
            Offer::Sandbox => Quota::Limited(20),
            Offer::Standard => Quota::Limited(100),
            Offer::Scale => Quota::Limited(500),
            Offer::Private => Quota::Unlimited,
        },
        QuotaKind::Webhooks => match offer {
            Offer::Free => Quota::Limited(5),
            Offer::Sandbox | Offer::Standard | Offer::Scale | Offer::Private => Quota::Limited(25),
        },
    }
}

pub fn resolve(
    organisation_override: Option<Quota>,
    offer: Option<Offer>,
    kind: QuotaKind,
) -> Quota {
    organisation_override.unwrap_or_else(|| default_quota(offer, kind))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct OfferDefault {
    pub offer: Offer,
    pub quota: Quota,
}

/// One kind as the platform panel shows it for an organisation.
///
/// The default is listed per offer rather than as one value: an organisation
/// can hold deployments on several offers, and the override replaces the
/// default of each of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct OrganisationQuota {
    pub kind: QuotaKind,
    pub description: &'static str,
    pub enforced: bool,
    pub defaults: Vec<OfferDefault>,
    pub organisation_override: Option<Quota>,
}

impl OrganisationQuota {
    pub fn new(kind: QuotaKind, organisation_override: Option<Quota>) -> Self {
        Self {
            kind,
            description: kind.description(),
            enforced: kind.is_enforced(),
            defaults: Offer::ALL
                .into_iter()
                .map(|offer| OfferDefault {
                    offer,
                    quota: default_quota(Some(offer), kind),
                })
                .collect(),
            organisation_override,
        }
    }
}

/// One kind as a deployment is held to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct EffectiveQuota {
    pub kind: QuotaKind,
    pub quota: Quota,
    pub overridden: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_key() {
        for kind in QuotaKind::ALL {
            assert_eq!(kind.as_str().parse::<QuotaKind>().unwrap(), kind);
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }

    #[test]
    fn the_key_on_the_wire_is_the_key_in_the_column() {
        for kind in QuotaKind::ALL {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(kind.as_str().to_string())
            );
        }
    }

    #[test]
    fn no_two_kinds_share_a_key() {
        let mut keys: Vec<_> = QuotaKind::ALL.iter().map(QuotaKind::as_str).collect();
        let before = keys.len();
        keys.sort();
        keys.dedup();

        assert_eq!(keys.len(), before);
    }

    #[test]
    fn a_key_nobody_defined_is_refused() {
        assert!(matches!(
            "nope".parse::<QuotaKind>(),
            Err(CoreError::UnknownQuota { .. })
        ));
    }

    #[test]
    fn every_kind_has_a_default_for_every_offer() {
        for kind in QuotaKind::ALL {
            for offer in Offer::ALL {
                let _ = default_quota(Some(offer), kind);
            }
            assert_eq!(default_quota(None, kind), Quota::Unlimited, "{kind}");
        }
    }

    #[test]
    fn the_free_offer_is_held_to_the_beta_grid() {
        assert_eq!(
            default_quota(Some(Offer::Free), QuotaKind::McpServers),
            Quota::Limited(1)
        );
        assert_eq!(
            default_quota(Some(Offer::Free), QuotaKind::Policies),
            Quota::Limited(5)
        );
        assert_eq!(
            default_quota(Some(Offer::Free), QuotaKind::Webhooks),
            Quota::Limited(5)
        );
    }

    #[test]
    fn the_webhook_defaults_are_what_the_plan_gave() {
        for offer in Offer::ALL {
            let plan_gave = if offer == Offer::Free { 5 } else { 25 };

            assert_eq!(
                default_quota(Some(offer), QuotaKind::Webhooks),
                Quota::Limited(plan_gave),
                "{offer}"
            );
        }
    }

    #[test]
    fn a_private_deployment_has_no_object_limit() {
        for kind in [QuotaKind::McpServers, QuotaKind::Policies] {
            assert_eq!(default_quota(Some(Offer::Private), kind), Quota::Unlimited);
        }
    }

    #[test]
    fn users_are_never_limited_by_default() {
        for offer in Offer::ALL {
            assert_eq!(
                default_quota(Some(offer), QuotaKind::Users),
                Quota::Unlimited
            );
        }
        assert!(!QuotaKind::Users.is_enforced());
    }

    #[test]
    fn a_limit_allows_up_to_its_value_and_no_further() {
        let two = Quota::Limited(2);

        assert!(two.allows(0));
        assert!(two.allows(1));
        assert!(!two.allows(2));
        assert!(!two.allows(3));
    }

    #[test]
    fn a_limit_of_zero_allows_nothing() {
        assert!(!Quota::Limited(0).allows(0));
    }

    #[test]
    fn unlimited_allows_anything() {
        assert!(Quota::Unlimited.allows(usize::MAX));
    }

    #[test]
    fn an_override_beats_the_default_in_both_directions() {
        let kind = QuotaKind::McpServers;

        assert_eq!(
            resolve(Some(Quota::Limited(9)), Some(Offer::Free), kind),
            Quota::Limited(9)
        );
        assert_eq!(
            resolve(Some(Quota::Limited(1)), Some(Offer::Scale), kind),
            Quota::Limited(1)
        );
        assert_eq!(
            resolve(Some(Quota::Unlimited), Some(Offer::Free), kind),
            Quota::Unlimited
        );
    }

    #[test]
    fn without_an_override_the_offer_decides() {
        assert_eq!(
            resolve(None, Some(Offer::Free), QuotaKind::Policies),
            Quota::Limited(5)
        );
    }

    #[test]
    fn an_override_applies_to_a_deployment_that_has_no_offer() {
        assert_eq!(
            resolve(Some(Quota::Limited(2)), None, QuotaKind::Policies),
            Quota::Limited(2)
        );
    }

    #[test]
    fn a_quota_is_tagged_on_the_wire() {
        assert_eq!(
            serde_json::to_value(Quota::Limited(3)).unwrap(),
            serde_json::json!({ "type": "limited", "max": 3 })
        );
        assert_eq!(
            serde_json::to_value(Quota::Unlimited).unwrap(),
            serde_json::json!({ "type": "unlimited" })
        );
        assert_eq!(
            serde_json::from_value::<Quota>(serde_json::json!({ "type": "limited", "max": 0 }))
                .unwrap(),
            Quota::Limited(0)
        );
        assert_eq!(
            serde_json::from_value::<Quota>(serde_json::json!({ "type": "unlimited" })).unwrap(),
            Quota::Unlimited
        );
    }

    #[test]
    fn no_override_and_an_unlimited_override_are_told_apart() {
        let none = OrganisationQuota::new(QuotaKind::Policies, None);
        let unlimited = OrganisationQuota::new(QuotaKind::Policies, Some(Quota::Unlimited));

        assert_ne!(
            serde_json::to_value(&none).unwrap()["organisation_override"],
            serde_json::to_value(&unlimited).unwrap()["organisation_override"]
        );
    }

    #[test]
    fn the_panel_lists_a_default_for_every_offer() {
        let view = OrganisationQuota::new(QuotaKind::Policies, Some(Quota::Limited(7)));

        assert_eq!(view.defaults.len(), Offer::ALL.len());
        assert_eq!(view.organisation_override, Some(Quota::Limited(7)));
    }
}
