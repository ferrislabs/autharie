use std::{fmt, str::FromStr};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::{
        RefOr,
        schema::{ObjectBuilder, Schema, Type},
    },
};

use crate::billing::BillingError;

const FIRST_YEAR: i32 = 1970;
const LAST_YEAR: i32 = 9999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BillingPeriod {
    year: i32,
    month: u32,
}

impl BillingPeriod {
    pub fn new(year: i32, month: u32) -> Result<Self, BillingError> {
        if !(FIRST_YEAR..=LAST_YEAR).contains(&year) || !(1..=12).contains(&month) {
            return Err(BillingError::InvalidPeriod {
                value: format!("{year:04}-{month:02}"),
            });
        }

        Ok(Self { year, month })
    }

    pub fn containing(instant: DateTime<Utc>) -> Self {
        Self {
            year: instant.year(),
            month: instant.month(),
        }
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u32 {
        self.month
    }

    pub fn starts_at(self) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(self.year, self.month, 1)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .map(|midnight| midnight.and_utc())
            .unwrap_or_default()
    }

    pub fn ends_at(self) -> DateTime<Utc> {
        self.next().starts_at()
    }

    pub fn seconds(self) -> i64 {
        self.ends_at().timestamp() - self.starts_at().timestamp()
    }

    pub fn next(self) -> Self {
        if self.month == 12 {
            Self {
                year: self.year + 1,
                month: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    pub fn previous(self) -> Self {
        if self.month == 1 {
            Self {
                year: self.year - 1,
                month: 12,
            }
        } else {
            Self {
                year: self.year,
                month: self.month - 1,
            }
        }
    }

    pub fn has_ended(self, now: DateTime<Utc>) -> bool {
        now >= self.ends_at()
    }
}

impl PartialSchema for BillingPeriod {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .pattern(Some(r"^[0-9]{4}-[0-9]{2}$"))
            .examples([serde_json::json!("2026-11")])
            .into()
    }
}

impl ToSchema for BillingPeriod {}

impl fmt::Display for BillingPeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl FromStr for BillingPeriod {
    type Err = BillingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || BillingError::InvalidPeriod {
            value: s.to_string(),
        };

        let bytes = s.as_bytes();
        let shaped = bytes.len() == 7
            && bytes[4] == b'-'
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..].iter().all(u8::is_ascii_digit);

        if !shaped {
            return Err(invalid());
        }

        let year = s[..4].parse::<i32>().map_err(|_| invalid())?;
        let month = s[5..].parse::<u32>().map_err(|_| invalid())?;

        Self::new(year, month).map_err(|_| invalid())
    }
}

impl TryFrom<String> for BillingPeriod {
    type Error = BillingError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<BillingPeriod> for String {
    fn from(value: BillingPeriod) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .unwrap()
    }

    #[test]
    fn accepts_a_year_and_a_month() {
        let period = BillingPeriod::new(2026, 11).unwrap();

        assert_eq!(period.year(), 2026);
        assert_eq!(period.month(), 11);
    }

    #[test]
    fn refuses_a_month_outside_one_to_twelve() {
        assert!(BillingPeriod::new(2026, 0).is_err());
        assert!(BillingPeriod::new(2026, 13).is_err());
    }

    #[test]
    fn refuses_a_year_outside_the_supported_range() {
        assert!(BillingPeriod::new(1969, 12).is_err());
        assert!(BillingPeriod::new(10000, 1).is_err());
        assert!(BillingPeriod::new(1970, 1).is_ok());
        assert!(BillingPeriod::new(9999, 12).is_ok());
    }

    #[test]
    fn parses_exactly_four_digits_a_dash_and_two_digits() {
        assert_eq!(
            "2026-11".parse::<BillingPeriod>().unwrap(),
            BillingPeriod::new(2026, 11).unwrap()
        );
        assert_eq!(
            "1970-01".parse::<BillingPeriod>().unwrap(),
            BillingPeriod::new(1970, 1).unwrap()
        );
    }

    #[test]
    fn refuses_any_other_shape() {
        for raw in [
            "",
            "2026",
            "2026-1",
            "2026-011",
            "26-11",
            "2026/11",
            "2026-13",
            "2026-00",
            "202a-11",
            "2026-1a",
            " 2026-11",
            "2026-11 ",
            "+026-11",
            "2026--1",
            "10000-01",
            "1969-12",
            "２０２６-１１",
        ] {
            assert_eq!(
                raw.parse::<BillingPeriod>(),
                Err(BillingError::InvalidPeriod {
                    value: raw.to_string()
                }),
                "{raw}"
            );
        }
    }

    #[test]
    fn displays_with_zero_padding() {
        assert_eq!(BillingPeriod::new(2026, 3).unwrap().to_string(), "2026-03");
        assert_eq!(BillingPeriod::new(1970, 12).unwrap().to_string(), "1970-12");
    }

    #[test]
    fn round_trips_through_serde_as_a_string() {
        let period = BillingPeriod::new(2026, 11).unwrap();

        assert_eq!(serde_json::to_string(&period).unwrap(), "\"2026-11\"");
        assert_eq!(
            serde_json::from_str::<BillingPeriod>("\"2026-11\"").unwrap(),
            period
        );
        assert!(serde_json::from_str::<BillingPeriod>("\"2026-13\"").is_err());
    }

    #[test]
    fn contains_the_instant_it_was_built_from() {
        assert_eq!(
            BillingPeriod::containing(at(2026, 11, 30, 23, 59, 59)),
            BillingPeriod::new(2026, 11).unwrap()
        );
        assert_eq!(
            BillingPeriod::containing(at(2026, 12, 1, 0, 0, 0)),
            BillingPeriod::new(2026, 12).unwrap()
        );
    }

    #[test]
    fn starts_at_midnight_utc_on_the_first() {
        assert_eq!(
            BillingPeriod::new(2026, 11).unwrap().starts_at(),
            at(2026, 11, 1, 0, 0, 0)
        );
    }

    #[test]
    fn ends_where_the_next_period_starts() {
        let november = BillingPeriod::new(2026, 11).unwrap();

        assert_eq!(november.ends_at(), at(2026, 12, 1, 0, 0, 0));
        assert_eq!(november.ends_at(), november.next().starts_at());
        assert_eq!(
            BillingPeriod::new(2026, 12).unwrap().ends_at(),
            at(2027, 1, 1, 0, 0, 0)
        );
    }

    #[test]
    fn counts_the_seconds_of_each_month_length() {
        assert_eq!(BillingPeriod::new(2026, 11).unwrap().seconds(), 30 * 86_400);
        assert_eq!(BillingPeriod::new(2026, 12).unwrap().seconds(), 31 * 86_400);
        assert_eq!(BillingPeriod::new(2027, 2).unwrap().seconds(), 28 * 86_400);
        assert_eq!(BillingPeriod::new(2028, 2).unwrap().seconds(), 29 * 86_400);
    }

    #[test]
    fn moves_across_year_boundaries() {
        let december = BillingPeriod::new(2026, 12).unwrap();
        let january = BillingPeriod::new(2027, 1).unwrap();

        assert_eq!(december.next(), january);
        assert_eq!(january.previous(), december);
        assert_eq!(january.next().previous(), january);
    }

    #[test]
    fn orders_chronologically() {
        let december = BillingPeriod::new(2026, 12).unwrap();
        let january = BillingPeriod::new(2027, 1).unwrap();
        let february = BillingPeriod::new(2027, 2).unwrap();

        assert!(december < january);
        assert!(january < february);
    }

    #[test]
    fn has_ended_from_the_first_instant_of_the_next_period() {
        let november = BillingPeriod::new(2026, 11).unwrap();

        assert!(!november.has_ended(at(2026, 11, 30, 23, 59, 59)));
        assert!(november.has_ended(at(2026, 12, 1, 0, 0, 0)));
        assert!(november.has_ended(at(2027, 1, 1, 0, 0, 0)));
    }
}
