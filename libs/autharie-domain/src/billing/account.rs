use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{billing::BillingError, organisation::OrganisationId};

const PROVIDER_REFERENCE_MAX_LEN: usize = 255;
const NAME_MAX_LEN: usize = 200;
const EMAIL_MAX_LEN: usize = 254;
const ADDRESS_FIELD_MAX_LEN: usize = 200;

pub(super) fn provider_reference(raw: &str) -> Result<String, BillingError> {
    let value = raw.trim();

    if value.is_empty()
        || value.len() > PROVIDER_REFERENCE_MAX_LEN
        || value.chars().any(char::is_whitespace)
    {
        return Err(BillingError::InvalidProviderReference {
            value: raw.to_string(),
        });
    }

    Ok(value.to_string())
}

fn invalid(field: &'static str, reason: &'static str) -> BillingError {
    BillingError::InvalidBillingDetails { field, reason }
}

fn bounded(raw: &str, field: &'static str, max_chars: usize) -> Result<String, BillingError> {
    let value = raw.trim();

    if value.is_empty() {
        return Err(invalid(field, "must not be empty"));
    }

    if value.chars().count() > max_chars {
        return Err(invalid(field, "is too long"));
    }

    if value.chars().any(char::is_control) {
        return Err(invalid(field, "must not contain control characters"));
    }

    Ok(value.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderCustomerId(String);

impl ProviderCustomerId {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        provider_reference(raw).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingName(String);

impl BillingName {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        bounded(raw, "name", NAME_MAX_LEN).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingEmail(String);

impl BillingEmail {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        let value = raw.trim();

        if value.is_empty() {
            return Err(invalid("email", "must not be empty"));
        }

        if value.len() > EMAIL_MAX_LEN {
            return Err(invalid("email", "is too long"));
        }

        if value.chars().any(char::is_whitespace) {
            return Err(invalid("email", "must not contain whitespace"));
        }

        if value.chars().any(char::is_control) {
            return Err(invalid("email", "must not contain control characters"));
        }

        let mut parts = value.split('@');
        let (local, domain) = match (parts.next(), parts.next(), parts.next()) {
            (Some(local), Some(domain), None) => (local, domain),
            _ => return Err(invalid("email", "must contain exactly one @")),
        };

        if local.is_empty() {
            return Err(invalid("email", "must have a name before the @"));
        }

        if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
            return Err(invalid("email", "must have a domain with a dot inside"));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryCode(String);

impl CountryCode {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        if raw.len() != 2 || !raw.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(invalid("country", "must be two letters"));
        }

        Ok(Self(raw.to_ascii_uppercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VatNumber(String);

impl VatNumber {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        let value = raw.replace(' ', "").to_ascii_uppercase();
        let bytes = value.as_bytes();

        let shaped = (4..=15).contains(&bytes.len())
            && bytes[..2].iter().all(u8::is_ascii_alphabetic)
            && bytes[2..].iter().all(u8::is_ascii_alphanumeric);

        if !shaped {
            return Err(invalid(
                "vat_number",
                "must be two letters then 2 to 13 letters or digits",
            ));
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Siren(String);

impl Siren {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        let value = raw.replace(' ', "");

        if value.len() != 9 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid("siren", "must be 9 digits"));
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BillingAddressInput<'a> {
    pub line1: &'a str,
    pub line2: Option<&'a str>,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub country: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingAddress {
    line1: String,
    line2: Option<String>,
    postal_code: String,
    city: String,
    country: CountryCode,
}

impl BillingAddress {
    pub fn parse(input: BillingAddressInput<'_>) -> Result<Self, BillingError> {
        let line2 = match input.line2.map(str::trim) {
            None | Some("") => None,
            Some(value) => Some(bounded(value, "line2", ADDRESS_FIELD_MAX_LEN)?),
        };

        Ok(Self {
            line1: bounded(input.line1, "line1", ADDRESS_FIELD_MAX_LEN)?,
            line2,
            postal_code: bounded(input.postal_code, "postal_code", ADDRESS_FIELD_MAX_LEN)?,
            city: bounded(input.city, "city", ADDRESS_FIELD_MAX_LEN)?,
            country: CountryCode::parse(input.country)?,
        })
    }

    pub fn line1(&self) -> &str {
        &self.line1
    }

    pub fn line2(&self) -> Option<&str> {
        self.line2.as_deref()
    }

    pub fn postal_code(&self) -> &str {
        &self.postal_code
    }

    pub fn city(&self) -> &str {
        &self.city
    }

    pub fn country(&self) -> &CountryCode {
        &self.country
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingDetails {
    pub name: BillingName,
    pub email: BillingEmail,
    pub address: BillingAddress,
    pub vat_number: Option<VatNumber>,
    pub siren: Option<Siren>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethodStatus {
    Missing,
    Valid,
    Failing,
}

impl fmt::Display for PaymentMethodStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("missing"),
            Self::Valid => f.write_str("valid"),
            Self::Failing => f.write_str("failing"),
        }
    }
}

impl FromStr for PaymentMethodStatus {
    type Err = BillingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "missing" => Ok(Self::Missing),
            "valid" => Ok(Self::Valid),
            "failing" => Ok(Self::Failing),
            _ => Err(BillingError::InvalidPaymentMethodStatus {
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingAccount {
    pub organisation_id: OrganisationId,
    pub details: BillingDetails,
    pub customer: Option<ProviderCustomerId>,
    pub payment_method: PaymentMethodStatus,
}

impl BillingAccount {
    pub fn new(organisation_id: OrganisationId, details: BillingDetails) -> Self {
        Self {
            organisation_id,
            details,
            customer: None,
            payment_method: PaymentMethodStatus::Missing,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.payment_method == PaymentMethodStatus::Valid
    }

    pub fn can_be_invoiced(&self) -> bool {
        self.customer.is_some() && self.payment_method != PaymentMethodStatus::Missing
    }
}

pub fn require_complete(account: Option<&BillingAccount>) -> Result<(), BillingError> {
    match account {
        Some(account) if account.is_complete() => Ok(()),
        _ => Err(BillingError::AccountIncomplete),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details() -> BillingDetails {
        BillingDetails {
            name: BillingName::parse("Acme SAS").unwrap(),
            email: BillingEmail::parse("billing@acme.test").unwrap(),
            address: BillingAddress::parse(BillingAddressInput {
                line1: "1 rue de la Paix",
                line2: None,
                postal_code: "75002",
                city: "Paris",
                country: "fr",
            })
            .unwrap(),
            vat_number: None,
            siren: None,
        }
    }

    fn account(customer: bool, payment_method: PaymentMethodStatus) -> BillingAccount {
        BillingAccount {
            customer: customer.then(|| ProviderCustomerId::parse("cus_123").unwrap()),
            payment_method,
            ..BillingAccount::new(OrganisationId::new(), details())
        }
    }

    #[test]
    fn a_provider_reference_is_trimmed() {
        assert_eq!(
            ProviderCustomerId::parse("  cus_123 ").unwrap().as_str(),
            "cus_123"
        );
    }

    #[test]
    fn a_provider_reference_refuses_blank_spaced_and_long_values() {
        for raw in ["", "   ", "cus 123", "cus\t123", &"c".repeat(256)] {
            assert!(
                matches!(
                    ProviderCustomerId::parse(raw),
                    Err(BillingError::InvalidProviderReference { .. })
                ),
                "{raw:?}"
            );
        }

        assert!(ProviderCustomerId::parse(&"c".repeat(255)).is_ok());
    }

    #[test]
    fn a_name_is_trimmed_and_bounded() {
        assert_eq!(BillingName::parse("  Acme  ").unwrap().as_str(), "Acme");
        assert!(BillingName::parse("   ").is_err());
        assert!(BillingName::parse(&"é".repeat(200)).is_ok());
        assert!(BillingName::parse(&"é".repeat(201)).is_err());
    }

    #[test]
    fn an_email_accepts_an_ordinary_address() {
        assert_eq!(
            BillingEmail::parse("  billing@acme.test ")
                .unwrap()
                .as_str(),
            "billing@acme.test"
        );
        assert!(BillingEmail::parse("a@b.c").is_ok());
        assert!(BillingEmail::parse("first.last+tag@mail.acme.test").is_ok());
    }

    #[test]
    fn an_email_refuses_a_malformed_address() {
        for raw in [
            "",
            "   ",
            "acme.test",
            "@acme.test",
            "a@@acme.test",
            "a@b@acme.test",
            "a@acme",
            "a@.acme",
            "a@acme.",
            "a@.",
            "a b@acme.test",
            "a@ac me.test",
        ] {
            assert!(
                matches!(
                    BillingEmail::parse(raw),
                    Err(BillingError::InvalidBillingDetails { field: "email", .. })
                ),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn an_email_refuses_a_control_character() {
        for raw in [
            "a\u{0}@acme.test",
            "a@acme\u{7f}.test",
            "a\u{1b}[0m@acme.test",
        ] {
            assert!(
                matches!(
                    BillingEmail::parse(raw),
                    Err(BillingError::InvalidBillingDetails { field: "email", .. })
                ),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn a_name_refuses_a_control_character() {
        for raw in ["Ac\nme", "Ac\tme", "Ac\u{0}me", "Ac\u{7f}me", "Ac\u{85}me"] {
            assert!(
                matches!(
                    BillingName::parse(raw),
                    Err(BillingError::InvalidBillingDetails { field: "name", .. })
                ),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn every_address_field_refuses_a_control_character() {
        let input = BillingAddressInput {
            line1: "1 rue de la Paix",
            line2: Some("Bat. B"),
            postal_code: "75002",
            city: "Paris",
            country: "FR",
        };
        let hostile = "x\ny";

        let cases = [
            (
                BillingAddressInput {
                    line1: hostile,
                    ..input
                },
                "line1",
            ),
            (
                BillingAddressInput {
                    line2: Some(hostile),
                    ..input
                },
                "line2",
            ),
            (
                BillingAddressInput {
                    postal_code: hostile,
                    ..input
                },
                "postal_code",
            ),
            (
                BillingAddressInput {
                    city: hostile,
                    ..input
                },
                "city",
            ),
        ];

        for (input, expected) in cases {
            assert!(
                matches!(
                    BillingAddress::parse(input),
                    Err(BillingError::InvalidBillingDetails { field, .. }) if field == expected
                ),
                "{expected}"
            );
        }
    }

    #[test]
    fn a_control_character_around_a_value_is_trimmed_not_refused() {
        assert_eq!(BillingName::parse("\tAcme\n").unwrap().as_str(), "Acme");
    }

    #[test]
    fn an_email_is_bounded_to_254_characters() {
        let local = "a".repeat(244);

        assert!(BillingEmail::parse(&format!("{local}@acme.test")).is_ok());
        assert!(BillingEmail::parse(&format!("{local}a@acme.test")).is_err());
    }

    #[test]
    fn a_country_code_is_two_letters_stored_uppercase() {
        assert_eq!(CountryCode::parse("fr").unwrap().as_str(), "FR");
        assert_eq!(CountryCode::parse("De").unwrap().as_str(), "DE");
    }

    #[test]
    fn a_country_code_refuses_anything_else() {
        for raw in ["", "f", "fra", "f1", "1f", "f ", " f", "éé"] {
            assert!(CountryCode::parse(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn a_vat_number_loses_its_spaces_and_is_uppercased() {
        assert_eq!(
            VatNumber::parse("fr 12 345 678 901").unwrap().as_str(),
            "FR12345678901"
        );
        assert_eq!(
            VatNumber::parse("DE123456789").unwrap().as_str(),
            "DE123456789"
        );
        assert_eq!(
            VatNumber::parse("ESX1234567Z").unwrap().as_str(),
            "ESX1234567Z"
        );
    }

    #[test]
    fn a_vat_number_refuses_a_malformed_value() {
        for raw in [
            "",
            "FR",
            "FR1",
            "12345678901",
            "F112345678901",
            "FR-12345678901",
            "FR12345678901234",
            "FR1234567890é",
        ] {
            assert!(
                matches!(
                    VatNumber::parse(raw),
                    Err(BillingError::InvalidBillingDetails {
                        field: "vat_number",
                        ..
                    })
                ),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn a_vat_number_accepts_its_shortest_and_longest_forms() {
        assert!(VatNumber::parse("FR12").is_ok());
        assert!(VatNumber::parse("FR1234567890123").is_ok());
    }

    #[test]
    fn a_siren_is_nine_digits_with_spaces_removed() {
        assert_eq!(Siren::parse("123 456 789").unwrap().as_str(), "123456789");
        assert_eq!(Siren::parse("123456789").unwrap().as_str(), "123456789");
    }

    #[test]
    fn a_siren_refuses_anything_else() {
        for raw in [
            "",
            "12345678",
            "1234567890",
            "12345678a",
            "123-456-789",
            "١٢٣٤٥٦٧٨٩",
        ] {
            assert!(
                matches!(
                    Siren::parse(raw),
                    Err(BillingError::InvalidBillingDetails { field: "siren", .. })
                ),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn an_address_is_trimmed_and_its_country_uppercased() {
        let address = BillingAddress::parse(BillingAddressInput {
            line1: "  1 rue de la Paix ",
            line2: Some(" Bâtiment B "),
            postal_code: " 75002",
            city: "Paris ",
            country: "fr",
        })
        .unwrap();

        assert_eq!(address.line1(), "1 rue de la Paix");
        assert_eq!(address.line2(), Some("Bâtiment B"));
        assert_eq!(address.postal_code(), "75002");
        assert_eq!(address.city(), "Paris");
        assert_eq!(address.country().as_str(), "FR");
    }

    #[test]
    fn a_blank_second_address_line_becomes_none() {
        for line2 in [None, Some(""), Some("   ")] {
            let address = BillingAddress::parse(BillingAddressInput {
                line1: "1 rue de la Paix",
                line2,
                postal_code: "75002",
                city: "Paris",
                country: "FR",
            })
            .unwrap();

            assert_eq!(address.line2(), None);
        }
    }

    #[test]
    fn an_address_names_the_field_it_refuses() {
        let input = BillingAddressInput {
            line1: "1 rue de la Paix",
            line2: None,
            postal_code: "75002",
            city: "Paris",
            country: "FR",
        };

        let cases = [
            (
                BillingAddressInput {
                    line1: " ",
                    ..input
                },
                "line1",
            ),
            (
                BillingAddressInput {
                    postal_code: "",
                    ..input
                },
                "postal_code",
            ),
            (
                BillingAddressInput {
                    city: "  ",
                    ..input
                },
                "city",
            ),
            (
                BillingAddressInput {
                    country: "France",
                    ..input
                },
                "country",
            ),
        ];

        for (input, expected) in cases {
            assert!(
                matches!(
                    BillingAddress::parse(input),
                    Err(BillingError::InvalidBillingDetails { field, .. }) if field == expected
                ),
                "{expected}"
            );
        }
    }

    #[test]
    fn an_address_line_is_bounded_to_200_characters() {
        let long = "a".repeat(201);
        let address = BillingAddress::parse(BillingAddressInput {
            line1: &long,
            line2: None,
            postal_code: "75002",
            city: "Paris",
            country: "FR",
        });

        assert!(address.is_err());
    }

    #[test]
    fn payment_method_status_round_trips_through_its_text() {
        for status in [
            PaymentMethodStatus::Missing,
            PaymentMethodStatus::Valid,
            PaymentMethodStatus::Failing,
        ] {
            assert_eq!(
                status.to_string().parse::<PaymentMethodStatus>(),
                Ok(status)
            );
        }

        assert_eq!(
            "expired".parse::<PaymentMethodStatus>(),
            Err(BillingError::InvalidPaymentMethodStatus {
                value: "expired".to_string()
            })
        );
    }

    #[test]
    fn a_new_account_has_no_customer_and_no_payment_method() {
        let account = BillingAccount::new(OrganisationId::new(), details());

        assert_eq!(account.customer, None);
        assert_eq!(account.payment_method, PaymentMethodStatus::Missing);
        assert!(!account.is_complete());
        assert!(!account.can_be_invoiced());
    }

    #[test]
    fn an_account_is_complete_only_with_a_valid_payment_method() {
        assert!(account(true, PaymentMethodStatus::Valid).is_complete());
        assert!(!account(true, PaymentMethodStatus::Failing).is_complete());
        assert!(!account(true, PaymentMethodStatus::Missing).is_complete());
    }

    #[test]
    fn an_account_can_be_invoiced_with_a_customer_and_a_payment_method_on_file() {
        assert!(account(true, PaymentMethodStatus::Valid).can_be_invoiced());
        assert!(account(true, PaymentMethodStatus::Failing).can_be_invoiced());
        assert!(!account(true, PaymentMethodStatus::Missing).can_be_invoiced());
        assert!(!account(false, PaymentMethodStatus::Valid).can_be_invoiced());
    }

    #[test]
    fn a_paid_tier_requires_a_complete_account() {
        let complete = account(true, PaymentMethodStatus::Valid);
        let failing = account(true, PaymentMethodStatus::Failing);

        assert_eq!(require_complete(Some(&complete)), Ok(()));
        assert_eq!(
            require_complete(Some(&failing)),
            Err(BillingError::AccountIncomplete)
        );
        assert_eq!(require_complete(None), Err(BillingError::AccountIncomplete));
    }
}
