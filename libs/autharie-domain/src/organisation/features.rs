use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::CoreError;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IamFeature {
    SsoConnectors,
    Mfa,
    DirectoryFederation,
    CustomDomain,
    Branding,
    Analytics,
    Compliance,
    DelegatedAdmin,
}

impl IamFeature {
    pub const ALL: [Self; 8] = [
        Self::SsoConnectors,
        Self::Mfa,
        Self::DirectoryFederation,
        Self::CustomDomain,
        Self::Branding,
        Self::Analytics,
        Self::Compliance,
        Self::DelegatedAdmin,
    ];
}

impl fmt::Display for IamFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SsoConnectors => write!(f, "sso_connectors"),
            Self::Mfa => write!(f, "mfa"),
            Self::DirectoryFederation => write!(f, "directory_federation"),
            Self::CustomDomain => write!(f, "custom_domain"),
            Self::Branding => write!(f, "branding"),
            Self::Analytics => write!(f, "analytics"),
            Self::Compliance => write!(f, "compliance"),
            Self::DelegatedAdmin => write!(f, "delegated_admin"),
        }
    }
}

impl std::str::FromStr for IamFeature {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|feature| feature.to_string() == value)
            .ok_or_else(|| CoreError::UnknownFeature {
                value: value.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_round_trips_through_its_name() {
        for feature in IamFeature::ALL {
            assert_eq!(feature.to_string().parse::<IamFeature>().unwrap(), feature);
        }
        assert!("sso".parse::<IamFeature>().is_err());
    }

    #[test]
    fn every_feature_is_listed() {
        for feature in IamFeature::ALL {
            match feature {
                IamFeature::SsoConnectors
                | IamFeature::Mfa
                | IamFeature::DirectoryFederation
                | IamFeature::CustomDomain
                | IamFeature::Branding
                | IamFeature::Analytics
                | IamFeature::Compliance
                | IamFeature::DelegatedAdmin => {}
            }
        }

        let mut sorted = IamFeature::ALL;
        sorted.sort();
        sorted
            .windows(2)
            .for_each(|pair| assert_ne!(pair[0], pair[1]));
        assert_eq!(IamFeature::ALL.len(), 8);
    }

    #[test]
    fn the_wire_names_are_snake_case() {
        let names: Vec<String> = IamFeature::ALL
            .into_iter()
            .map(|feature| {
                serde_json::to_value(feature)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let displayed: Vec<String> = IamFeature::ALL.into_iter().map(|f| f.to_string()).collect();
        assert_eq!(names, displayed);
    }
}
