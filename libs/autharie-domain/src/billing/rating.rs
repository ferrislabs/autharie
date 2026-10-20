use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::billing::{
    BillingPeriod, Money,
    statement::StatementLine,
    subscription::{Segment, Subscription},
};

pub fn rate(
    period: BillingPeriod,
    first_period: BillingPeriod,
    subscriptions: &[Subscription],
) -> Vec<StatementLine> {
    if period < first_period {
        return Vec::new();
    }

    lines_until(period, subscriptions, period.ends_at())
}

pub fn estimate(
    period: BillingPeriod,
    first_period: BillingPeriod,
    subscriptions: &[Subscription],
    now: DateTime<Utc>,
) -> Vec<StatementLine> {
    if period < first_period || now <= period.starts_at() {
        return Vec::new();
    }

    lines_until(period, subscriptions, period.ends_at().min(now))
}

fn lines_until(
    period: BillingPeriod,
    subscriptions: &[Subscription],
    until: DateTime<Utc>,
) -> Vec<StatementLine> {
    let mut lines: Vec<StatementLine> = subscriptions
        .iter()
        .filter(|subscription| subscription.terms().is_billable())
        .filter_map(|subscription| line_of(subscription, period, until))
        .collect();

    lines.sort_by_key(|line| {
        (
            line.deployment_id.0,
            line.starts_at,
            line.ends_at,
            line.subscription_id,
        )
    });

    lines
}

fn line_of(
    subscription: &Subscription,
    period: BillingPeriod,
    until: DateTime<Utc>,
) -> Option<StatementLine> {
    let segment = subscription
        .segment(period.starts_at(), until)
        .filter(|segment| segment.seconds() > 0)?;
    let covered_seconds = segment.seconds();
    let terms = subscription.terms();

    Some(StatementLine {
        deployment_id: subscription.deployment_id(),
        subscription_id: subscription.id(),
        tier: terms.tier.clone(),
        monthly_price: terms.monthly_price,
        starts_at: segment.starts_at,
        ends_at: segment.ends_at,
        covered_seconds,
        amount: terms
            .monthly_price
            .prorate(covered_seconds, period.seconds()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct FirstInvoice {
    pub period: BillingPeriod,
    pub amount: Money,
}

pub fn first_invoice(
    monthly_price: Money,
    opened_at: DateTime<Utc>,
    first_period: BillingPeriod,
) -> FirstInvoice {
    let period = BillingPeriod::containing(opened_at);

    if period < first_period {
        return FirstInvoice {
            period: first_period,
            amount: monthly_price,
        };
    }

    let covered = Segment {
        starts_at: opened_at,
        ends_at: period.ends_at(),
    };

    FirstInvoice {
        period,
        amount: monthly_price.prorate(covered.seconds(), period.seconds()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::{Duration, TimeZone};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;
    use crate::{
        billing::{SubscriptionId, SubscriptionParts, SubscriptionTerms, TierCode},
        deployments::DeploymentId,
        offers::Offer,
        organisation::OrganisationId,
    };

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .unwrap()
    }

    fn november() -> BillingPeriod {
        BillingPeriod::new(2026, 11).unwrap()
    }

    fn january() -> BillingPeriod {
        BillingPeriod::new(2026, 1).unwrap()
    }

    fn terms(price: i64) -> SubscriptionTerms {
        SubscriptionTerms {
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(price),
            offer: Offer::Standard,
            features: BTreeSet::new(),
        }
    }

    fn deployment(n: u128) -> DeploymentId {
        DeploymentId(Uuid::from_u128(n))
    }

    fn subscription(
        deployment_id: DeploymentId,
        price: i64,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
    ) -> Subscription {
        Subscription::try_from(SubscriptionParts {
            id: SubscriptionId::new(),
            organisation_id: OrganisationId(Uuid::from_u128(1)),
            deployment_id,
            terms: terms(price),
            starts_at,
            ends_at,
        })
        .unwrap()
    }

    fn open(price: i64, starts_at: DateTime<Utc>) -> Subscription {
        subscription(deployment(7), price, starts_at, None)
    }

    #[test]
    fn a_subscription_opened_mid_month_is_invoiced_for_the_seconds_left() {
        let subscriptions = [open(4900, at(2026, 11, 10, 12, 30, 15))];

        let lines = rate(november(), january(), &subscriptions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].starts_at, at(2026, 11, 10, 12, 30, 15));
        assert_eq!(lines[0].ends_at, at(2026, 12, 1, 0, 0, 0));
        assert_eq!(lines[0].covered_seconds, 1_769_385);
        assert_eq!(lines[0].monthly_price, Money::from_minor(4900));
        assert_eq!(lines[0].amount, Money::from_minor(3345));
        assert_eq!(lines[0].deployment_id, deployment(7));
        assert_eq!(lines[0].subscription_id, subscriptions[0].id());
        assert_eq!(lines[0].tier, TierCode::legacy(Offer::Standard));
    }

    #[test]
    fn a_subscription_held_less_than_a_second_gives_no_line() {
        let starts_at = at(2026, 11, 10, 10, 0, 0) + chrono::Duration::milliseconds(200);
        let ends_at = at(2026, 11, 10, 10, 0, 0) + chrono::Duration::milliseconds(700);
        let subscriptions = [subscription(deployment(7), 4900, starts_at, Some(ends_at))];

        assert!(rate(november(), january(), &subscriptions).is_empty());
    }

    #[test]
    fn a_subscription_held_all_month_is_invoiced_its_whole_price() {
        let subscriptions = [open(4900, at(2026, 9, 3, 8, 0, 0))];

        let lines = rate(november(), january(), &subscriptions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].covered_seconds, november().seconds());
        assert_eq!(lines[0].amount, Money::from_minor(4900));
        assert_eq!(lines[0].starts_at, november().starts_at());
        assert_eq!(lines[0].ends_at, november().ends_at());
    }

    #[test]
    fn a_mid_month_upgrade_gives_two_lines_each_at_its_own_price() {
        let before = subscription(deployment(7), 2900, at(2026, 10, 1, 0, 0, 0), None);
        let switch = at(2026, 11, 10, 8, 0, 0);
        let after = before.successor(terms(9900), switch);
        let before = before.close(switch).unwrap();

        let lines = rate(november(), january(), &[after, before]);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].monthly_price, Money::from_minor(2900));
        assert_eq!(lines[0].covered_seconds, 806_400);
        assert_eq!(lines[0].amount, Money::from_minor(902));
        assert_eq!(lines[1].monthly_price, Money::from_minor(9900));
        assert_eq!(lines[1].covered_seconds, 1_785_600);
        assert_eq!(lines[1].amount, Money::from_minor(6820));
        assert_eq!(
            lines.iter().map(|line| line.covered_seconds).sum::<i64>(),
            november().seconds()
        );
        assert_eq!(lines[0].ends_at, lines[1].starts_at);
    }

    #[test]
    fn a_deployment_deleted_mid_month_is_invoiced_until_that_instant() {
        let deleted = at(2026, 11, 20, 6, 0, 0);
        let subscriptions = [subscription(
            deployment(7),
            4900,
            at(2026, 10, 5, 0, 0, 0),
            Some(deleted),
        )];

        let lines = rate(november(), january(), &subscriptions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].ends_at, deleted);
        assert_eq!(lines[0].covered_seconds, 1_663_200);
        assert_eq!(lines[0].amount, Money::from_minor(3144));
    }

    #[test]
    fn an_organisation_with_only_free_subscriptions_gets_no_line() {
        let subscriptions = [
            open(0, at(2026, 10, 1, 0, 0, 0)),
            subscription(deployment(8), 0, at(2026, 11, 2, 0, 0, 0), None),
        ];

        assert_eq!(rate(november(), january(), &subscriptions), Vec::new());
    }

    #[test]
    fn a_free_subscription_never_hides_a_paid_one() {
        let paid = subscription(deployment(8), 4900, at(2026, 10, 1, 0, 0, 0), None);
        let subscriptions = [open(0, at(2026, 10, 1, 0, 0, 0)), paid.clone()];

        let lines = rate(november(), january(), &subscriptions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].subscription_id, paid.id());
    }

    #[test]
    fn a_period_before_the_first_period_produces_no_line() {
        let subscriptions = [open(4900, at(2026, 9, 1, 0, 0, 0))];

        assert_eq!(
            rate(november(), november().next(), &subscriptions),
            Vec::new()
        );
        assert_eq!(
            rate(november().previous(), november(), &subscriptions),
            Vec::new()
        );
    }

    #[test]
    fn the_first_period_itself_is_invoiced() {
        let subscriptions = [open(4900, at(2026, 9, 1, 0, 0, 0))];

        let lines = rate(november(), november(), &subscriptions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].amount, Money::from_minor(4900));
    }

    #[test]
    fn time_spent_stopped_is_not_invoiced() {
        let stopped = at(2026, 11, 10, 0, 0, 0);
        let restarted = at(2026, 11, 20, 0, 0, 0);
        let running = subscription(deployment(7), 4900, at(2026, 10, 1, 0, 0, 0), None);
        let resumed = running.successor(terms(4900), restarted);
        let running = running.close(stopped).unwrap();

        let lines = rate(november(), january(), &[running, resumed]);

        let gap = (restarted - stopped).num_seconds();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].covered_seconds, 777_600);
        assert_eq!(lines[1].covered_seconds, 950_400);
        assert_eq!(
            lines.iter().map(|line| line.covered_seconds).sum::<i64>(),
            november().seconds() - gap
        );
        assert_eq!(lines[0].amount, Money::from_minor(1470));
        assert_eq!(lines[1].amount, Money::from_minor(1797));
    }

    #[test]
    fn an_exact_half_rounds_up() {
        let subscriptions = [open(4901, at(2026, 11, 16, 0, 0, 0))];

        let lines = rate(november(), january(), &subscriptions);

        assert_eq!(lines[0].covered_seconds, 1_296_000);
        assert_eq!(lines[0].amount, Money::from_minor(2451));
    }

    #[test]
    fn just_under_a_half_rounds_down_and_a_half_rounds_up() {
        let opened = at(2026, 11, 30, 16, 48, 0);

        let below = rate(november(), january(), &[open(4949, opened)]);
        let half = rate(november(), january(), &[open(4950, opened)]);

        assert_eq!(below[0].covered_seconds, 25_920);
        assert_eq!(below[0].amount, Money::from_minor(49));
        assert_eq!(half[0].amount, Money::from_minor(50));
    }

    #[test]
    fn each_line_is_rounded_once_on_its_own() {
        let switch = at(2026, 11, 16, 0, 0, 0);
        let first = subscription(deployment(7), 4901, at(2026, 10, 1, 0, 0, 0), None);
        let second = first.successor(terms(4901), switch);
        let first = first.close(switch).unwrap();

        let lines = rate(november(), january(), &[first, second]);

        assert_eq!(lines[0].amount, Money::from_minor(2451));
        assert_eq!(lines[1].amount, Money::from_minor(2451));
        assert_eq!(
            lines.iter().map(|line| line.amount).sum::<Money>(),
            Money::from_minor(4902)
        );
    }

    #[test]
    fn a_subscription_that_ended_before_the_period_or_starts_after_it_gives_no_line() {
        let subscriptions = [
            subscription(
                deployment(7),
                4900,
                at(2026, 9, 1, 0, 0, 0),
                Some(at(2026, 11, 1, 0, 0, 0)),
            ),
            open(4900, at(2026, 12, 1, 0, 0, 0)),
        ];

        assert_eq!(rate(november(), january(), &subscriptions), Vec::new());
    }

    #[test]
    fn lines_are_sorted_by_deployment_then_start() {
        let late = subscription(deployment(2), 4900, at(2026, 11, 20, 0, 0, 0), None);
        let early = subscription(
            deployment(2),
            4900,
            at(2026, 11, 2, 0, 0, 0),
            Some(at(2026, 11, 10, 0, 0, 0)),
        );
        let other = subscription(deployment(1), 4900, at(2026, 11, 25, 0, 0, 0), None);

        let lines = rate(
            november(),
            january(),
            &[late.clone(), early.clone(), other.clone()],
        );

        let order: Vec<SubscriptionId> = lines.iter().map(|line| line.subscription_id).collect();
        assert_eq!(order, vec![other.id(), early.id(), late.id()]);
    }

    #[test]
    fn an_estimate_cuts_every_segment_at_now_and_divides_by_the_whole_period() {
        let subscriptions = [open(4900, at(2026, 10, 1, 0, 0, 0))];

        let lines = estimate(
            november(),
            january(),
            &subscriptions,
            at(2026, 11, 16, 0, 0, 0),
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].ends_at, at(2026, 11, 16, 0, 0, 0));
        assert_eq!(lines[0].covered_seconds, 1_296_000);
        assert_eq!(lines[0].amount, Money::from_minor(2450));
    }

    #[test]
    fn an_estimate_leaves_a_segment_that_ended_before_now_alone() {
        let ended = at(2026, 11, 10, 0, 0, 0);
        let subscriptions = [subscription(
            deployment(7),
            4900,
            at(2026, 10, 1, 0, 0, 0),
            Some(ended),
        )];

        let lines = estimate(
            november(),
            january(),
            &subscriptions,
            at(2026, 11, 16, 0, 0, 0),
        );

        assert_eq!(lines[0].ends_at, ended);
        assert_eq!(lines[0].covered_seconds, 777_600);
    }

    #[test]
    fn an_estimate_is_empty_until_the_period_has_started() {
        let subscriptions = [open(4900, at(2026, 10, 1, 0, 0, 0))];

        assert_eq!(
            estimate(
                november(),
                january(),
                &subscriptions,
                november().starts_at()
            ),
            Vec::new()
        );
        assert_eq!(
            estimate(
                november(),
                january(),
                &subscriptions,
                at(2026, 10, 15, 0, 0, 0)
            ),
            Vec::new()
        );
    }

    #[test]
    fn an_estimate_after_the_period_is_the_rating() {
        let subscriptions = [
            open(4900, at(2026, 11, 10, 12, 30, 15)),
            subscription(
                deployment(8),
                2900,
                at(2026, 10, 1, 0, 0, 0),
                Some(at(2026, 11, 20, 6, 0, 0)),
            ),
        ];

        assert_eq!(
            estimate(
                november(),
                january(),
                &subscriptions,
                at(2027, 3, 1, 0, 0, 0)
            ),
            rate(november(), january(), &subscriptions)
        );
    }

    #[test]
    fn an_estimate_before_the_first_period_is_empty() {
        let subscriptions = [open(4900, at(2026, 10, 1, 0, 0, 0))];

        assert_eq!(
            estimate(
                november(),
                november().next(),
                &subscriptions,
                at(2026, 11, 16, 0, 0, 0)
            ),
            Vec::new()
        );
    }

    #[test]
    fn a_subscription_opened_in_the_first_period_pays_the_rest_of_that_month() {
        let invoice = first_invoice(
            Money::from_minor(4900),
            at(2026, 11, 16, 0, 0, 0),
            november(),
        );

        assert_eq!(invoice.period, november());
        assert_eq!(invoice.amount, Money::from_minor(2450));
    }

    #[test]
    fn a_subscription_opened_after_the_first_period_pays_the_rest_of_its_own_month() {
        let invoice = first_invoice(
            Money::from_minor(4900),
            at(2026, 12, 10, 12, 0, 0),
            november(),
        );

        assert_eq!(invoice.period, BillingPeriod::new(2026, 12).unwrap());
        assert_eq!(invoice.amount, Money::from_minor(3398));
    }

    #[test]
    fn a_subscription_opened_before_the_first_period_pays_the_first_period_in_full() {
        let invoice = first_invoice(
            Money::from_minor(4900),
            at(2026, 10, 20, 9, 0, 0),
            november(),
        );

        assert_eq!(invoice.period, november());
        assert_eq!(invoice.amount, Money::from_minor(4900));
    }

    #[test]
    fn a_subscription_opened_at_the_first_instant_of_the_first_period_pays_in_full() {
        let invoice = first_invoice(Money::from_minor(4900), november().starts_at(), november());

        assert_eq!(invoice.period, november());
        assert_eq!(invoice.amount, Money::from_minor(4900));
    }

    #[derive(Debug, Clone)]
    struct Draw {
        deployment: u8,
        price: i64,
        start: i64,
        length: Option<i64>,
    }

    fn period_strategy() -> impl Strategy<Value = BillingPeriod> {
        (2024i32..=2030, 1u32..=12)
            .prop_map(|(year, month)| BillingPeriod::new(year, month).unwrap())
    }

    fn draw_strategy() -> impl Strategy<Value = Draw> {
        (
            0u8..4,
            0i64..=20_000,
            -5_184_000i64..=7_776_000,
            prop::option::of(0i64..=5_000_000),
        )
            .prop_map(|(deployment, price, start, length)| Draw {
                deployment,
                price,
                start,
                length,
            })
    }

    fn build(period: BillingPeriod, draws: &[Draw]) -> Vec<Subscription> {
        draws
            .iter()
            .enumerate()
            .map(|(index, draw)| {
                let starts_at = period.starts_at() + Duration::seconds(draw.start);

                Subscription::try_from(SubscriptionParts {
                    id: SubscriptionId(Uuid::from_u128(index as u128 + 1)),
                    organisation_id: OrganisationId(Uuid::from_u128(1)),
                    deployment_id: deployment(u128::from(draw.deployment) + 1),
                    terms: terms(draw.price),
                    starts_at,
                    ends_at: draw
                        .length
                        .map(|length| starts_at + Duration::seconds(length)),
                })
                .unwrap()
            })
            .collect()
    }

    fn scenario() -> impl Strategy<Value = (BillingPeriod, Vec<Subscription>, Vec<Subscription>)> {
        (
            period_strategy(),
            prop::collection::vec(draw_strategy(), 0..8),
        )
            .prop_flat_map(|(period, draws)| {
                let subscriptions = build(period, &draws);

                (
                    Just(period),
                    Just(subscriptions.clone()),
                    Just(subscriptions).prop_shuffle(),
                )
            })
    }

    proptest! {
        #[test]
        fn rating_is_deterministic_whatever_the_input_order(
            (period, ordered, shuffled) in scenario(),
            now_offset in -1_000_000i64..=4_000_000,
        ) {
            let first_period = period.previous();
            let now = period.starts_at() + Duration::seconds(now_offset);

            let lines = rate(period, first_period, &ordered);
            prop_assert_eq!(&lines, &rate(period, first_period, &ordered));
            prop_assert_eq!(&lines, &rate(period, first_period, &shuffled));

            let estimated = estimate(period, first_period, &ordered, now);
            prop_assert_eq!(&estimated, &estimate(period, first_period, &ordered, now));
            prop_assert_eq!(&estimated, &estimate(period, first_period, &shuffled, now));
        }

        #[test]
        fn contiguous_subscriptions_cover_the_period_exactly(
            period in period_strategy(),
            price in 1i64..=1_000_000,
            lead in 0i64..=1_000_000,
            tail in prop::option::of(0i64..=1_000_000),
            mut cuts in prop::collection::vec(0i64..=2_678_400, 0..8),
        ) {
            let seconds = period.seconds();
            for cut in &mut cuts {
                *cut = (*cut).min(seconds);
            }
            cuts.sort_unstable();

            let mut boundaries = vec![0];
            boundaries.extend(cuts);
            boundaries.push(seconds);

            let last = boundaries.len() - 2;
            let subscriptions: Vec<Subscription> = boundaries
                .windows(2)
                .enumerate()
                .map(|(index, window)| {
                    let starts_at = period.starts_at()
                        + Duration::seconds(window[0] - if index == 0 { lead } else { 0 });
                    let ends_at = if index == last {
                        tail.map(|tail| period.ends_at() + Duration::seconds(tail))
                    } else {
                        Some(period.starts_at() + Duration::seconds(window[1]))
                    };

                    subscription(deployment(7), price, starts_at, ends_at)
                })
                .collect();

            let lines = rate(period, period, &subscriptions);

            prop_assert_eq!(
                lines.iter().map(|line| line.covered_seconds).sum::<i64>(),
                seconds
            );

            let total: i64 = lines.iter().map(|line| line.amount.minor()).sum();
            prop_assert!((total - price).abs() <= lines.len() as i64);
        }

        #[test]
        fn non_overlapping_subscriptions_never_exceed_the_period(
            period in period_strategy(),
            begin in -3_000_000i64..=3_000_000,
            open_last in any::<bool>(),
            steps in prop::collection::vec(
                (0i64..=1_000_000, 0i64..=2_000_000, 0i64..=50_000),
                0..8,
            ),
        ) {
            let mut cursor = period.starts_at() + Duration::seconds(begin);
            let last = steps.len().saturating_sub(1);
            let mut subscriptions = Vec::new();

            for (index, (gap, length, price)) in steps.iter().enumerate() {
                let starts_at = cursor + Duration::seconds(*gap);
                let ends_at = starts_at + Duration::seconds(*length);
                cursor = ends_at;

                let closing = (!(open_last && index == last)).then_some(ends_at);
                subscriptions.push(subscription(deployment(7), *price, starts_at, closing));
            }

            let lines = rate(period, period, &subscriptions);

            prop_assert!(
                lines.iter().map(|line| line.covered_seconds).sum::<i64>() <= period.seconds()
            );

            for line in &lines {
                prop_assert!(line.amount >= Money::ZERO);
                prop_assert!(line.amount <= line.monthly_price);
                prop_assert!(line.covered_seconds <= period.seconds());
            }
        }
    }
}
