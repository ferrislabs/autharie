use std::{iter::Sum, ops::Add};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Self = Self(0);

    pub const fn from_minor(minor: i64) -> Self {
        Self(minor)
    }

    pub const fn minor(self) -> i64 {
        self.0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub fn prorate(self, covered_seconds: i64, period_seconds: i64) -> Self {
        if period_seconds <= 0 {
            return Self::ZERO;
        }

        let covered = i128::from(covered_seconds.clamp(0, period_seconds));
        let period = i128::from(period_seconds);
        let product = i128::from(self.0) * covered;
        let magnitude = (2 * product.abs() + period) / (2 * period);
        let rounded = if product < 0 { -magnitude } else { magnitude };

        let bounded = rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
        Self(i64::try_from(bounded).unwrap_or_default())
    }
}

impl Add for Money {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl Sum for Money {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Add::add)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_not_positive() {
        assert!(!Money::ZERO.is_positive());
        assert!(!Money::from_minor(-1).is_positive());
        assert!(Money::from_minor(1).is_positive());
    }

    #[test]
    fn minor_units_round_trip() {
        assert_eq!(Money::from_minor(4900).minor(), 4900);
        assert_eq!(Money::default(), Money::ZERO);
    }

    #[test]
    fn prorate_rounds_an_exact_half_up() {
        assert_eq!(Money::from_minor(1).prorate(1, 2), Money::from_minor(1));
        assert_eq!(Money::from_minor(5).prorate(1, 2), Money::from_minor(3));
        assert_eq!(Money::from_minor(150).prorate(1, 100), Money::from_minor(2));
    }

    #[test]
    fn prorate_rounds_below_a_half_down() {
        assert_eq!(Money::from_minor(49).prorate(1, 100), Money::ZERO);
        assert_eq!(Money::from_minor(149).prorate(1, 100), Money::from_minor(1));
    }

    #[test]
    fn prorate_rounds_a_negative_half_away_from_zero() {
        assert_eq!(Money::from_minor(-5).prorate(1, 2), Money::from_minor(-3));
        assert_eq!(
            Money::from_minor(-149).prorate(1, 100),
            Money::from_minor(-1)
        );
    }

    #[test]
    fn prorate_keeps_the_whole_price_for_the_whole_period() {
        assert_eq!(
            Money::from_minor(4900).prorate(2_592_000, 2_592_000),
            Money::from_minor(4900)
        );
    }

    #[test]
    fn prorate_clamps_the_covered_seconds_to_the_period() {
        assert_eq!(Money::from_minor(4900).prorate(-10, 100), Money::ZERO);
        assert_eq!(
            Money::from_minor(4900).prorate(1_000, 100),
            Money::from_minor(4900)
        );
    }

    #[test]
    fn prorate_over_an_empty_period_is_zero() {
        assert_eq!(Money::from_minor(4900).prorate(10, 0), Money::ZERO);
        assert_eq!(Money::from_minor(4900).prorate(10, -5), Money::ZERO);
    }

    #[test]
    fn prorate_does_not_overflow_on_large_amounts() {
        assert_eq!(
            Money::from_minor(i64::MAX).prorate(i64::MAX, i64::MAX),
            Money::from_minor(i64::MAX)
        );
        assert_eq!(
            Money::from_minor(i64::MIN).prorate(i64::MAX, i64::MAX),
            Money::from_minor(i64::MIN)
        );
    }

    #[test]
    fn sums_add_up_and_empty_sums_are_zero() {
        let total: Money = [100, 250, 5].into_iter().map(Money::from_minor).sum();

        assert_eq!(total, Money::from_minor(355));
        assert_eq!(std::iter::empty::<Money>().sum::<Money>(), Money::ZERO);
        assert_eq!(
            Money::from_minor(2) + Money::from_minor(3),
            Money::from_minor(5)
        );
    }

    #[test]
    fn serialises_as_a_bare_integer() {
        assert_eq!(
            serde_json::to_string(&Money::from_minor(4900)).unwrap(),
            "4900"
        );
        assert_eq!(
            serde_json::from_str::<Money>("4900").unwrap(),
            Money::from_minor(4900)
        );
    }
}
