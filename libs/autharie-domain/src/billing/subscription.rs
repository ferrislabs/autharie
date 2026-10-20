use std::{collections::BTreeSet, fmt};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    billing::{BillingError, Money, tier::TierCode},
    deployments::DeploymentId,
    generate_uuid_v7,
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
pub struct SubscriptionId(pub Uuid);

impl SubscriptionId {
    pub fn new() -> Self {
        Self(generate_uuid_v7())
    }
}

impl Default for SubscriptionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for SubscriptionId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SubscriptionTerms {
    pub tier: TierCode,
    pub monthly_price: Money,
    pub offer: Offer,
    pub features: BTreeSet<IamFeature>,
}

impl SubscriptionTerms {
    pub fn is_billable(&self) -> bool {
        self.monthly_price.is_positive()
    }

    pub fn opens(&self, feature: IamFeature) -> bool {
        self.features.contains(&feature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Subscription {
    id: SubscriptionId,
    organisation_id: OrganisationId,
    deployment_id: DeploymentId,
    terms: SubscriptionTerms,
    starts_at: DateTime<Utc>,
    ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionParts {
    pub id: SubscriptionId,
    pub organisation_id: OrganisationId,
    pub deployment_id: DeploymentId,
    pub terms: SubscriptionTerms,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
}

impl TryFrom<SubscriptionParts> for Subscription {
    type Error = BillingError;

    fn try_from(parts: SubscriptionParts) -> Result<Self, Self::Error> {
        if parts
            .ends_at
            .is_some_and(|ends_at| ends_at < parts.starts_at)
        {
            return Err(BillingError::SubscriptionEndsBeforeStart);
        }

        Ok(Self {
            id: parts.id,
            organisation_id: parts.organisation_id,
            deployment_id: parts.deployment_id,
            terms: parts.terms,
            starts_at: parts.starts_at,
            ends_at: parts.ends_at,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl Segment {
    pub fn seconds(&self) -> i64 {
        self.ends_at.timestamp() - self.starts_at.timestamp()
    }
}

impl Subscription {
    pub fn open(
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        terms: SubscriptionTerms,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: SubscriptionId::new(),
            organisation_id,
            deployment_id,
            terms,
            starts_at: at,
            ends_at: None,
        }
    }

    pub fn close(self, at: DateTime<Utc>) -> Result<Self, BillingError> {
        if self.ends_at.is_some() {
            return Err(BillingError::SubscriptionClosed { id: self.id.0 });
        }

        if at < self.starts_at {
            return Err(BillingError::SubscriptionEndsBeforeStart);
        }

        Ok(Self {
            ends_at: Some(at),
            ..self
        })
    }

    pub fn successor(&self, terms: SubscriptionTerms, at: DateTime<Utc>) -> Self {
        Self::open(self.organisation_id, self.deployment_id, terms, at)
    }

    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    pub fn organisation_id(&self) -> OrganisationId {
        self.organisation_id
    }

    pub fn deployment_id(&self) -> DeploymentId {
        self.deployment_id
    }

    pub fn terms(&self) -> &SubscriptionTerms {
        &self.terms
    }

    pub fn starts_at(&self) -> DateTime<Utc> {
        self.starts_at
    }

    pub fn ends_at(&self) -> Option<DateTime<Utc>> {
        self.ends_at
    }

    pub fn is_open(&self) -> bool {
        self.ends_at.is_none()
    }

    pub fn is_valid_at(&self, at: DateTime<Utc>) -> bool {
        self.starts_at <= at && self.ends_at.is_none_or(|ends_at| at < ends_at)
    }

    pub fn segment(&self, from: DateTime<Utc>, until: DateTime<Utc>) -> Option<Segment> {
        let starts_at = self.starts_at.max(from);
        let ends_at = self.ends_at.map_or(until, |ends_at| ends_at.min(until));

        (starts_at < ends_at).then_some(Segment { starts_at, ends_at })
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
    }

    fn terms(price: i64) -> SubscriptionTerms {
        SubscriptionTerms {
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(price),
            offer: Offer::Standard,
            features: BTreeSet::from([IamFeature::Mfa]),
        }
    }

    fn open_at(day: u32) -> Subscription {
        Subscription::open(
            OrganisationId::new(),
            DeploymentId(Uuid::new_v4()),
            terms(4900),
            at(day, 0),
        )
    }

    #[test]
    fn a_fresh_subscription_is_open_from_its_instant() {
        let subscription = open_at(5);

        assert!(subscription.is_open());
        assert_eq!(subscription.starts_at(), at(5, 0));
        assert_eq!(subscription.ends_at(), None);
        assert_eq!(subscription.terms(), &terms(4900));
    }

    #[test]
    fn closing_sets_the_end_and_keeps_the_identity() {
        let subscription = open_at(5);
        let id = subscription.id();

        let closed = subscription.close(at(9, 12)).unwrap();

        assert_eq!(closed.id(), id);
        assert_eq!(closed.ends_at(), Some(at(9, 12)));
        assert!(!closed.is_open());
    }

    #[test]
    fn closing_at_the_start_instant_is_allowed() {
        let closed = open_at(5).close(at(5, 0)).unwrap();

        assert_eq!(closed.ends_at(), Some(at(5, 0)));
    }

    #[test]
    fn closing_twice_names_the_subscription() {
        let closed = open_at(5).close(at(9, 0)).unwrap();
        let id = closed.id();

        assert_eq!(
            closed.close(at(10, 0)),
            Err(BillingError::SubscriptionClosed { id: id.0 })
        );
    }

    #[test]
    fn closing_before_the_start_is_refused() {
        assert_eq!(
            open_at(5).close(at(4, 23)),
            Err(BillingError::SubscriptionEndsBeforeStart)
        );
    }

    #[test]
    fn a_successor_is_a_new_open_subscription_of_the_same_deployment() {
        let first = open_at(5);

        let next = first.successor(terms(9900), at(12, 0));

        assert_ne!(next.id(), first.id());
        assert_eq!(next.organisation_id(), first.organisation_id());
        assert_eq!(next.deployment_id(), first.deployment_id());
        assert_eq!(next.terms(), &terms(9900));
        assert_eq!(next.starts_at(), at(12, 0));
        assert!(next.is_open());
    }

    #[test]
    fn parts_ending_before_they_start_are_refused() {
        let subscription = open_at(5);

        let parts = SubscriptionParts {
            id: subscription.id(),
            organisation_id: subscription.organisation_id(),
            deployment_id: subscription.deployment_id(),
            terms: terms(4900),
            starts_at: at(5, 0),
            ends_at: Some(at(4, 0)),
        };

        assert_eq!(
            Subscription::try_from(parts),
            Err(BillingError::SubscriptionEndsBeforeStart)
        );
    }

    #[test]
    fn parts_ending_where_they_start_are_accepted() {
        let subscription = open_at(5);

        let parts = SubscriptionParts {
            id: subscription.id(),
            organisation_id: subscription.organisation_id(),
            deployment_id: subscription.deployment_id(),
            terms: terms(4900),
            starts_at: at(5, 0),
            ends_at: Some(at(5, 0)),
        };

        assert!(Subscription::try_from(parts).is_ok());
    }

    #[test]
    fn parts_rebuild_the_subscription_they_came_from() {
        let closed = open_at(5).close(at(9, 0)).unwrap();

        let parts = SubscriptionParts {
            id: closed.id(),
            organisation_id: closed.organisation_id(),
            deployment_id: closed.deployment_id(),
            terms: closed.terms().clone(),
            starts_at: closed.starts_at(),
            ends_at: closed.ends_at(),
        };

        assert_eq!(Subscription::try_from(parts), Ok(closed));
    }

    #[test]
    fn an_open_subscription_is_valid_from_its_start_on() {
        let subscription = open_at(5);

        assert!(!subscription.is_valid_at(at(4, 23)));
        assert!(subscription.is_valid_at(at(5, 0)));
        assert!(subscription.is_valid_at(at(30, 0)));
    }

    #[test]
    fn a_closed_subscription_is_valid_until_its_end_exclusive() {
        let closed = open_at(5).close(at(9, 0)).unwrap();

        assert!(closed.is_valid_at(at(8, 23)));
        assert!(!closed.is_valid_at(at(9, 0)));
    }

    #[test]
    fn the_segment_of_an_open_subscription_runs_to_the_end_of_the_window() {
        let segment = open_at(5).segment(at(1, 0), at(30, 0)).unwrap();

        assert_eq!(segment.starts_at, at(5, 0));
        assert_eq!(segment.ends_at, at(30, 0));
        assert_eq!(segment.seconds(), 25 * 86_400);
    }

    #[test]
    fn the_segment_of_a_closed_subscription_stops_at_its_end() {
        let closed = open_at(5).close(at(9, 0)).unwrap();

        let segment = closed.segment(at(1, 0), at(30, 0)).unwrap();

        assert_eq!(segment.starts_at, at(5, 0));
        assert_eq!(segment.ends_at, at(9, 0));
    }

    #[test]
    fn the_segment_is_cut_to_the_window() {
        let closed = open_at(5).close(at(25, 0)).unwrap();

        let segment = closed.segment(at(10, 0), at(20, 0)).unwrap();

        assert_eq!(segment.starts_at, at(10, 0));
        assert_eq!(segment.ends_at, at(20, 0));
    }

    #[test]
    fn there_is_no_segment_outside_the_window() {
        let closed = open_at(5).close(at(9, 0)).unwrap();

        assert_eq!(closed.segment(at(9, 0), at(30, 0)), None);
        assert_eq!(closed.segment(at(10, 0), at(30, 0)), None);
        assert_eq!(open_at(20).segment(at(1, 0), at(20, 0)), None);
    }

    #[test]
    fn a_subscription_closed_at_its_start_has_no_segment() {
        let closed = open_at(5).close(at(5, 0)).unwrap();

        assert_eq!(closed.segment(at(1, 0), at(30, 0)), None);
    }

    #[test]
    fn terms_open_the_features_they_list() {
        let terms = terms(4900);

        assert!(terms.opens(IamFeature::Mfa));
        assert!(!terms.opens(IamFeature::Branding));
    }

    #[test]
    fn terms_with_a_positive_price_are_billable() {
        assert!(terms(1).is_billable());
        assert!(!terms(0).is_billable());
    }

    #[test]
    fn segment_seconds_telescope_across_fractional_instants() {
        let first = Utc.timestamp_opt(100, 600_000_000).unwrap();
        let middle = Utc.timestamp_opt(200, 300_000_000).unwrap();
        let last = Utc.timestamp_opt(300, 900_000_000).unwrap();

        let left = Segment {
            starts_at: first,
            ends_at: middle,
        };
        let right = Segment {
            starts_at: middle,
            ends_at: last,
        };
        let whole = Segment {
            starts_at: first,
            ends_at: last,
        };

        assert_eq!(left.seconds() + right.seconds(), whole.seconds());
    }
}
