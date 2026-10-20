//! What an organisation may choose from, and what it would take to choose the
//! rest.
//!
//! The whole catalogue is answered rather than only the open part. A screen
//! that receives three offers cannot say what the fourth would cost, and
//! hiding an offer makes an upsell invisible; showing it with what opens it
//! turns a refusal into a next step.

use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    dataplane::value_objects::DeploymentResources,
    offers::{DatabasePlacement, Offer},
    organisation::value_objects::Plan,
};

/// One offer, seen from an organisation on a given tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct OfferAvailability {
    pub offer: Offer,

    /// What an instance on it is given.
    pub resources: DeploymentResources,

    /// Whether it shares a cluster with other organisations.
    pub shares_a_cluster: bool,

    /// Whether this organisation may choose it now.
    pub open: bool,

    /// The tier that would open it, when this one does not.
    ///
    /// Absent when it is already open. A field that named a tier beside an
    /// open offer would be read as an upsell on something already bought.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_by: Option<Plan>,
}

/// The whole catalogue, answered for one tier.
pub fn offers_for(plan: Plan) -> Vec<OfferAvailability> {
    Offer::ALL
        .into_iter()
        .filter(|offer| offer.database() == DatabasePlacement::Managed || offer.is_open_to(plan))
        .map(|offer| {
            let open = offer.is_open_to(plan);

            OfferAvailability {
                offer,
                resources: offer.resources(),
                shares_a_cluster: offer.mode()
                    == crate::dataplane::value_objects::DataPlaneMode::Shared,
                open,
                opened_by: (!open).then(|| offer.cheapest_tier()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole catalogue, so a screen can show what is not bought yet.
    #[test]
    fn every_dedicated_offer_is_answered_whatever_the_tier() {
        for plan in [Plan::Free, Plan::Starter, Plan::Business, Plan::Enterprise] {
            let answered: Vec<Offer> = offers_for(plan).iter().map(|entry| entry.offer).collect();

            for offer in Offer::ALL
                .into_iter()
                .filter(|offer| offer.database() == DatabasePlacement::Managed)
            {
                assert!(
                    answered.contains(&offer),
                    "{plan} is not told about {offer}"
                );
            }
        }
    }

    #[test]
    fn the_free_offer_is_shown_only_to_the_tier_it_is_for() {
        let shown = |plan| {
            offers_for(plan)
                .iter()
                .any(|entry| entry.offer == Offer::Free)
        };

        assert!(shown(Plan::Free));
        for plan in [Plan::Starter, Plan::Business, Plan::Enterprise] {
            assert!(!shown(plan), "{plan} was offered the free offer");
        }
    }

    #[test]
    fn a_free_organisation_sees_the_sandbox_locked_behind_starter() {
        let sandbox = offers_for(Plan::Free)
            .into_iter()
            .find(|entry| entry.offer == Offer::Sandbox)
            .expect("the catalogue is whole");

        assert!(!sandbox.open);
        assert_eq!(sandbox.opened_by, Some(Plan::Starter));
    }

    #[test]
    fn a_free_organisation_is_told_what_would_open_the_rest() {
        let catalogue = offers_for(Plan::Free);

        let private = catalogue
            .iter()
            .find(|entry| entry.offer == Offer::Private)
            .expect("the catalogue is whole");

        assert!(!private.open);
        assert_eq!(private.opened_by, Some(Plan::Enterprise));
    }

    /// A tier named beside an offer already open reads as an upsell on
    /// something the customer has already bought.
    #[test]
    fn an_open_offer_names_no_tier() {
        for entry in offers_for(Plan::Enterprise) {
            assert!(entry.open, "{} is closed to enterprise", entry.offer);
            assert_eq!(entry.opened_by, None);
        }
    }

    #[test]
    fn the_catalogue_says_which_offers_share_a_cluster() {
        let catalogue = offers_for(Plan::Enterprise);

        let shared: Vec<_> = catalogue
            .iter()
            .filter(|entry| entry.shares_a_cluster)
            .map(|entry| entry.offer)
            .collect();

        assert!(shared.contains(&Offer::Standard));
        assert!(!shared.contains(&Offer::Private));
    }
}
