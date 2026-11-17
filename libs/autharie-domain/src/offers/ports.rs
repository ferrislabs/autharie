use std::{collections::HashMap, future::Future};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    offers::{Offer, OfferAvailability, OrganisationOffer},
    organisation::OrganisationId,
};

pub trait OfferService: Send + Sync {
    /// The whole catalogue, answered for this organisation's tier.
    ///
    /// Gated on being able to see instances: the caller asking is the one
    /// about to place one, and the answer discloses which tier they are on.
    fn list_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<OfferAvailability>, CoreError>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferChange {
    pub by: String,
    pub at: DateTime<Utc>,
}

/// The offers platform staff opened or closed for one organisation. What the
/// plan opens is not stored: it lives in [`crate::offers::Offer::open_to`].
pub trait OfferOverrideRepository: Send + Sync {
    fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<HashMap<Offer, bool>, CoreError>> + Send;

    /// `None` removes the override, which returns the offer to the plan.
    fn set_override(
        &self,
        organisation_id: OrganisationId,
        offer: Offer,
        open: Option<bool>,
        change: &OfferChange,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait OrganisationOfferService: Send + Sync {
    fn list_organisation_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<OrganisationOffer>, CoreError>> + Send;

    fn set_organisation_offer(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        offer: Offer,
        open: Option<bool>,
    ) -> impl Future<Output = Result<OrganisationOffer, CoreError>> + Send;
}
