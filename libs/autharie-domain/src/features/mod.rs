pub mod ports;
pub mod service;

use std::{fmt, str::FromStr};

use serde::Serialize;
use utoipa::ToSchema;

use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    CustomerCloud,
}

impl Feature {
    pub const ALL: [Self; 1] = [Self::CustomerCloud];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CustomerCloud => "customer_cloud",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::CustomerCloud => {
                "Deploy into the customer's own cloud account (bring your own cloud)."
            }
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Feature {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|feature| feature.as_str() == value)
            .ok_or_else(|| CoreError::UnknownFeature {
                value: value.to_string(),
            })
    }
}

pub fn resolve(organisation_override: Option<bool>, default: Option<bool>) -> bool {
    organisation_override.or(default).unwrap_or(false)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct OverrideTally {
    pub on: usize,
    pub off: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct FeatureOverview {
    pub feature: Feature,
    pub enabled_by_default: bool,
    pub overrides: OverrideTally,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct OrganisationFeature {
    pub feature: Feature,
    pub enabled_by_default: bool,
    pub organisation_override: Option<bool>,
    pub effective: bool,
}

impl OrganisationFeature {
    pub fn new(
        feature: Feature,
        enabled_by_default: bool,
        organisation_override: Option<bool>,
    ) -> Self {
        Self {
            feature,
            enabled_by_default,
            organisation_override,
            effective: resolve(organisation_override, Some(enabled_by_default)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_round_trips_through_its_key() {
        for feature in Feature::ALL {
            assert_eq!(feature.as_str().parse::<Feature>().unwrap(), feature);
            assert_eq!(feature.to_string(), feature.as_str());
        }
    }

    #[test]
    fn the_key_on_the_wire_is_the_key_in_the_column() {
        for feature in Feature::ALL {
            assert_eq!(
                serde_json::to_value(feature).unwrap(),
                serde_json::Value::String(feature.as_str().to_string())
            );
        }
    }

    #[test]
    fn no_two_features_share_a_key() {
        let mut keys: Vec<_> = Feature::ALL.iter().map(Feature::as_str).collect();
        let before = keys.len();
        keys.sort();
        keys.dedup();

        assert_eq!(keys.len(), before);
    }

    #[test]
    fn a_key_nobody_defined_is_refused() {
        assert!(matches!(
            "nope".parse::<Feature>(),
            Err(CoreError::UnknownFeature { .. })
        ));
        assert!("".parse::<Feature>().is_err());
    }

    #[test]
    fn every_feature_says_what_it_does() {
        for feature in Feature::ALL {
            assert!(!feature.description().is_empty(), "{feature}");
        }
    }

    #[test]
    fn nothing_said_means_off() {
        assert!(!resolve(None, None));
    }

    #[test]
    fn the_default_applies_without_an_override() {
        assert!(resolve(None, Some(true)));
        assert!(!resolve(None, Some(false)));
    }

    #[test]
    fn an_override_beats_the_default_in_both_directions() {
        assert!(resolve(Some(true), Some(false)));
        assert!(!resolve(Some(false), Some(true)));
    }

    #[test]
    fn an_override_applies_when_the_default_was_never_set() {
        assert!(resolve(Some(true), None));
        assert!(!resolve(Some(false), None));
    }

    #[test]
    fn an_organisation_view_carries_the_resolved_value() {
        let feature = Feature::CustomerCloud;

        assert!(OrganisationFeature::new(feature, true, None).effective);
        assert!(!OrganisationFeature::new(feature, true, Some(false)).effective);
        assert!(OrganisationFeature::new(feature, false, Some(true)).effective);
    }
}
