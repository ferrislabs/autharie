use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::dataplane::{
    credential::{CloudCredentialId, CredentialError, ScopeCheck, SecretString},
    value_objects::Region,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Scaleway,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scaleway => "scaleway",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An amount per month in the minor unit of the provider's currency.
///
/// Integers on purpose: an estimate summed from floats shows a different
/// total depending on the order it was added in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
pub struct Money(u64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub fn new(minor_units: u64) -> Self {
        Self(minor_units)
    }

    pub fn minor_units(&self) -> u64 {
        self.0
    }

    /// Saturating: a total that cannot be represented must not wrap round to
    /// a small one and understate what the customer will pay.
    pub fn plus(self, other: Money) -> Money {
        Money(self.0.saturating_add(other.0))
    }

    pub fn times(self, count: u8) -> Money {
        Money(self.0.saturating_mul(u64::from(count)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct ControlPlaneOfferId(String);

impl ControlPlaneOfferId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneKind {
    Mutualized,
    Dedicated,
}

/// A control plane the provider sells in a region, as the catalog described
/// it. Its id and size are the provider's, never an enum kept here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ControlPlaneOffer {
    pub id: ControlPlaneOfferId,
    pub kind: ControlPlaneKind,
    pub monthly_price: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct NodeType(String);

impl NodeType {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct NodeOffer {
    pub node_type: NodeType,
    pub monthly_price: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ToSchema)]
pub struct ProviderOffers {
    pub control_planes: Vec<ControlPlaneOffer>,
    pub node_types: Vec<NodeOffer>,
}

impl ProviderOffers {
    pub fn control_plane(&self, id: &ControlPlaneOfferId) -> Option<&ControlPlaneOffer> {
        self.control_planes.iter().find(|offer| &offer.id == id)
    }

    pub fn node_offer(&self, node_type: &NodeType) -> Option<&NodeOffer> {
        self.node_types
            .iter()
            .find(|offer| &offer.node_type == node_type)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("the provider rejected the credential")]
    CredentialRejected,

    #[error("the provider does not serve region '{region}'")]
    RegionUnavailable { region: String },

    #[error("the provider catalog could not be read: {0}")]
    Unavailable(String),
}

/// What a provider sells in a region, read at runtime. Used to validate a
/// profile and to price it.
#[cfg_attr(test, mockall::automock)]
pub trait ProviderCatalog: Send + Sync {
    fn offers(
        &self,
        provider: Provider,
        credential_id: &CloudCredentialId,
        region: &Region,
    ) -> impl Future<Output = Result<ProviderOffers, CatalogError>> + Send;
}

/// Checks that a credential holds the permissions we require and no more.
#[cfg_attr(test, mockall::automock)]
pub trait CredentialVerifier: Send + Sync {
    fn verify(
        &self,
        provider: Provider,
        secret: &SecretString,
    ) -> impl Future<Output = Result<ScopeCheck, CredentialError>> + Send;
}
