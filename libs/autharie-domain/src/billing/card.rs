use crate::billing::BillingError;

const BRAND_MAX_LEN: usize = 32;
const LAST4_LEN: usize = 4;
const FIRST_YEAR: u32 = 2000;
const LAST_YEAR: u32 = 9999;

fn invalid(field: &'static str, reason: &'static str) -> BillingError {
    BillingError::InvalidBillingDetails { field, reason }
}

#[derive(Debug, Clone, Copy)]
pub struct CardSummaryInput<'a> {
    pub brand: &'a str,
    pub last4: &'a str,
    pub exp_month: u32,
    pub exp_year: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardSummary {
    brand: String,
    last4: String,
    exp_month: u8,
    exp_year: u16,
}

impl CardSummary {
    pub fn parse(input: CardSummaryInput<'_>) -> Result<Self, BillingError> {
        let brand = input.brand.trim().to_ascii_lowercase();

        if brand.is_empty()
            || brand.len() > BRAND_MAX_LEN
            || !brand
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(invalid(
                "brand",
                "must be 1 to 32 lowercase letters, digits or underscores",
            ));
        }

        if input.last4.len() != LAST4_LEN || !input.last4.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid("last4", "must be 4 digits"));
        }

        let exp_month = u8::try_from(input.exp_month)
            .ok()
            .filter(|month| (1..=12).contains(month))
            .ok_or_else(|| invalid("exp_month", "must be between 1 and 12"))?;

        let exp_year = u16::try_from(input.exp_year)
            .ok()
            .filter(|_| (FIRST_YEAR..=LAST_YEAR).contains(&input.exp_year))
            .ok_or_else(|| invalid("exp_year", "must be a four-digit year"))?;

        Ok(Self {
            brand,
            last4: input.last4.to_string(),
            exp_month,
            exp_year,
        })
    }

    pub fn brand(&self) -> &str {
        &self.brand
    }

    pub fn last4(&self) -> &str {
        &self.last4
    }

    pub fn exp_month(&self) -> u8 {
        self.exp_month
    }

    pub fn exp_year(&self) -> u16 {
        self.exp_year
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> CardSummaryInput<'static> {
        CardSummaryInput {
            brand: "visa",
            last4: "4242",
            exp_month: 4,
            exp_year: 2027,
        }
    }

    #[test]
    fn a_card_keeps_what_the_provider_reported() {
        let card = CardSummary::parse(input()).unwrap();

        assert_eq!(card.brand(), "visa");
        assert_eq!(card.last4(), "4242");
        assert_eq!(card.exp_month(), 4);
        assert_eq!(card.exp_year(), 2027);
    }

    #[test]
    fn a_brand_is_trimmed_and_lowercased() {
        let card = CardSummary::parse(CardSummaryInput {
            brand: "  MasterCard ",
            ..input()
        })
        .unwrap();

        assert_eq!(card.brand(), "mastercard");
    }

    #[test]
    fn a_brand_may_hold_an_underscore() {
        assert!(
            CardSummary::parse(CardSummaryInput {
                brand: "cartes_bancaires",
                ..input()
            })
            .is_ok()
        );
    }

    #[test]
    fn a_brand_refuses_blank_spaced_long_and_non_ascii_values() {
        for brand in ["", "   ", "vi sa", "visa!", "vïsa", &"v".repeat(33)] {
            assert!(
                matches!(
                    CardSummary::parse(CardSummaryInput { brand, ..input() }),
                    Err(BillingError::InvalidBillingDetails { field: "brand", .. })
                ),
                "{brand:?}"
            );
        }

        assert!(
            CardSummary::parse(CardSummaryInput {
                brand: &"v".repeat(32),
                ..input()
            })
            .is_ok()
        );
    }

    #[test]
    fn last4_is_exactly_four_ascii_digits() {
        for last4 in ["", "424", "42424", "42a4", " 424", "٤٢٤٢"] {
            assert!(
                matches!(
                    CardSummary::parse(CardSummaryInput { last4, ..input() }),
                    Err(BillingError::InvalidBillingDetails { field: "last4", .. })
                ),
                "{last4:?}"
            );
        }

        assert!(
            CardSummary::parse(CardSummaryInput {
                last4: "0007",
                ..input()
            })
            .is_ok()
        );
    }

    #[test]
    fn the_month_is_one_to_twelve() {
        for exp_month in [0, 13, 99] {
            assert!(
                matches!(
                    CardSummary::parse(CardSummaryInput {
                        exp_month,
                        ..input()
                    }),
                    Err(BillingError::InvalidBillingDetails {
                        field: "exp_month",
                        ..
                    })
                ),
                "{exp_month}"
            );
        }

        for exp_month in [1, 12] {
            assert!(
                CardSummary::parse(CardSummaryInput {
                    exp_month,
                    ..input()
                })
                .is_ok(),
                "{exp_month}"
            );
        }
    }

    #[test]
    fn the_year_is_a_four_digit_year() {
        for exp_year in [0, 27, 1999, 10_000, u32::MAX] {
            assert!(
                matches!(
                    CardSummary::parse(CardSummaryInput {
                        exp_year,
                        ..input()
                    }),
                    Err(BillingError::InvalidBillingDetails {
                        field: "exp_year",
                        ..
                    })
                ),
                "{exp_year}"
            );
        }

        for exp_year in [2000, 9999] {
            assert!(
                CardSummary::parse(CardSummaryInput {
                    exp_year,
                    ..input()
                })
                .is_ok(),
                "{exp_year}"
            );
        }
    }
}
