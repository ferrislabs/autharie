use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    billing::{BillingError, Money, subscription::SubscriptionTerms},
    deployments::{DeploymentKind, distribution::Distribution},
    offers::Offer,
    organisation::features::IamFeature,
};

const TIER_CODE_MAX_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, example = "legacy-standard")]
pub struct TierCode(String);

impl TierCode {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        let shaped = !raw.is_empty()
            && raw.len() <= TIER_CODE_MAX_LEN
            && raw.split('-').all(|group| {
                !group.is_empty()
                    && group
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });

        if !shaped {
            return Err(BillingError::InvalidTierCode {
                value: raw.to_string(),
            });
        }

        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn legacy(offer: Offer) -> Self {
        Self(format!("legacy-{offer}"))
    }
}

impl fmt::Display for TierCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for TierCode {
    type Err = BillingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for TierCode {
    type Error = BillingError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<TierCode> for String {
    fn from(value: TierCode) -> Self {
        value.0
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Hosting {
    Shared,
    SelfHosted,
    CustomerCloud,
}

impl Hosting {
    pub const ALL: [Self; 3] = [Self::Shared, Self::SelfHosted, Self::CustomerCloud];

    pub fn of(distribution: &Distribution) -> Self {
        match distribution {
            Distribution::Shared => Self::Shared,
            Distribution::SelfHosted => Self::SelfHosted,
            Distribution::CustomerCloud { .. } => Self::CustomerCloud,
        }
    }
}

impl fmt::Display for Hosting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shared => f.write_str("shared"),
            Self::SelfHosted => f.write_str("self_hosted"),
            Self::CustomerCloud => f.write_str("customer_cloud"),
        }
    }
}

impl FromStr for Hosting {
    type Err = BillingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "shared" => Ok(Self::Shared),
            "self_hosted" => Ok(Self::SelfHosted),
            "customer_cloud" => Ok(Self::CustomerCloud),
            _ => Err(BillingError::InvalidHosting {
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Tier {
    pub code: TierCode,
    pub name: String,
    pub monthly_price: Money,
    pub offer: Offer,
    pub features: BTreeSet<IamFeature>,
    pub engines: Vec<DeploymentKind>,
    pub hostings: Vec<Hosting>,
    pub selectable: bool,
}

impl Tier {
    pub fn is_billable(&self) -> bool {
        self.monthly_price.is_positive()
    }

    pub fn allows(&self, engine: &DeploymentKind, hosting: Hosting) -> Result<(), BillingError> {
        if !self.engines.contains(engine) {
            return Err(BillingError::TierDisallowsEngine {
                code: self.code.to_string(),
                engine: engine.to_string(),
            });
        }

        if !self.hostings.contains(&hosting) {
            return Err(BillingError::TierDisallowsHosting {
                code: self.code.to_string(),
                hosting: hosting.to_string(),
            });
        }

        Ok(())
    }

    pub fn terms(&self) -> SubscriptionTerms {
        SubscriptionTerms {
            tier: self.code.clone(),
            monthly_price: self.monthly_price,
            offer: self.offer,
            features: self.features.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier() -> Tier {
        Tier {
            code: TierCode::parse("standard-2026").unwrap(),
            name: "Standard".to_string(),
            monthly_price: Money::from_minor(4900),
            offer: Offer::Standard,
            features: BTreeSet::from([IamFeature::Mfa, IamFeature::Branding]),
            engines: vec![DeploymentKind::Ferriskey],
            hostings: vec![Hosting::Shared, Hosting::SelfHosted],
            selectable: true,
        }
    }

    #[test]
    fn tier_code_accepts_groups_of_lowercase_letters_and_digits() {
        for raw in ["a", "free", "standard-2026", "legacy-standard", "a-b-c-1"] {
            assert_eq!(TierCode::parse(raw).unwrap().as_str(), raw);
        }
    }

    #[test]
    fn tier_code_accepts_exactly_sixty_four_characters() {
        let longest = "a".repeat(64);

        assert!(TierCode::parse(&longest).is_ok());
        assert!(TierCode::parse(&format!("{longest}a")).is_err());
    }

    #[test]
    fn tier_code_refuses_anything_else() {
        for raw in [
            "", "-", "-a", "a-", "a--b", "A", "aB", "a b", "a_b", "a.b", "é", "a-é", " a", "a ",
        ] {
            assert_eq!(
                TierCode::parse(raw),
                Err(BillingError::InvalidTierCode {
                    value: raw.to_string()
                }),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn tier_code_of_a_legacy_offer_is_prefixed_with_legacy() {
        assert_eq!(
            TierCode::legacy(Offer::Standard).as_str(),
            "legacy-standard"
        );
        assert_eq!(TierCode::legacy(Offer::Free).as_str(), "legacy-free");

        for offer in Offer::ALL {
            assert!(TierCode::parse(TierCode::legacy(offer).as_str()).is_ok());
        }
    }

    #[test]
    fn tier_code_round_trips_through_serde_and_display() {
        let code = TierCode::parse("scale-2027").unwrap();

        assert_eq!(code.to_string(), "scale-2027");
        assert_eq!("scale-2027".parse::<TierCode>().unwrap(), code);
        assert_eq!(serde_json::to_string(&code).unwrap(), "\"scale-2027\"");
        assert_eq!(
            serde_json::from_str::<TierCode>("\"scale-2027\"").unwrap(),
            code
        );
        assert!(serde_json::from_str::<TierCode>("\"Scale\"").is_err());
    }

    #[test]
    fn hosting_round_trips_through_its_text() {
        for hosting in Hosting::ALL {
            assert_eq!(hosting.to_string().parse::<Hosting>().unwrap(), hosting);
        }

        assert_eq!(Hosting::SelfHosted.to_string(), "self_hosted");
        assert_eq!(Hosting::CustomerCloud.to_string(), "customer_cloud");
    }

    #[test]
    fn hosting_refuses_an_unknown_text() {
        assert_eq!(
            "cloud".parse::<Hosting>(),
            Err(BillingError::InvalidHosting {
                value: "cloud".to_string()
            })
        );
        assert!("Shared".parse::<Hosting>().is_err());
    }

    #[test]
    fn hosting_follows_the_distribution() {
        assert_eq!(Hosting::of(&Distribution::Shared), Hosting::Shared);
        assert_eq!(Hosting::of(&Distribution::SelfHosted), Hosting::SelfHosted);
    }

    #[test]
    fn a_tier_with_a_positive_price_is_billable() {
        let mut tier = tier();

        assert!(tier.is_billable());

        tier.monthly_price = Money::ZERO;

        assert!(!tier.is_billable());
    }

    #[test]
    fn a_tier_allows_an_engine_and_a_hosting_it_lists() {
        assert_eq!(
            tier().allows(&DeploymentKind::Ferriskey, Hosting::SelfHosted),
            Ok(())
        );
    }

    #[test]
    fn a_tier_refuses_an_engine_it_does_not_list() {
        assert_eq!(
            tier().allows(&DeploymentKind::Keycloak, Hosting::Shared),
            Err(BillingError::TierDisallowsEngine {
                code: "standard-2026".to_string(),
                engine: "keycloak".to_string()
            })
        );
    }

    #[test]
    fn a_tier_refuses_a_hosting_it_does_not_list() {
        assert_eq!(
            tier().allows(&DeploymentKind::Ferriskey, Hosting::CustomerCloud),
            Err(BillingError::TierDisallowsHosting {
                code: "standard-2026".to_string(),
                hosting: "customer_cloud".to_string()
            })
        );
    }

    #[test]
    fn a_tier_names_the_engine_before_the_hosting() {
        assert!(matches!(
            tier().allows(&DeploymentKind::Keycloak, Hosting::CustomerCloud),
            Err(BillingError::TierDisallowsEngine { .. })
        ));
    }

    #[test]
    fn terms_copy_the_commercial_fields_of_the_tier() {
        let tier = tier();
        let terms = tier.terms();

        assert_eq!(terms.tier, tier.code);
        assert_eq!(terms.monthly_price, tier.monthly_price);
        assert_eq!(terms.offer, tier.offer);
        assert_eq!(terms.features, tier.features);
    }
}
