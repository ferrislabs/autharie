use autharie_domain::dataplane::{
    cloud_provider::{CatalogError, CredentialVerifier, Provider, ProviderCatalog, ProviderOffers},
    credential::{CloudCredentialId, CredentialError, ScopeCheck, SecretString},
    value_objects::Region,
};
use chrono::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedVerdict {
    Accept,
    Invalid,
    Missing(Vec<String>),
    Excess(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedCloudProvider {
    pub offers: ProviderOffers,
    pub verdict: FixedVerdict,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CloudProviders {
    #[default]
    Unconfigured,
    Fixed(FixedCloudProvider),
}

const UNCONFIGURED: &str = "no cloud provider is configured on this installation";

impl ProviderCatalog for CloudProviders {
    async fn offers(
        &self,
        _provider: Provider,
        _credential_id: &CloudCredentialId,
        _region: &Region,
    ) -> Result<ProviderOffers, CatalogError> {
        match self {
            Self::Unconfigured => Err(CatalogError::Unavailable(UNCONFIGURED.to_string())),
            Self::Fixed(fixed) => Ok(fixed.offers.clone()),
        }
    }
}

impl CredentialVerifier for CloudProviders {
    async fn verify(
        &self,
        _provider: Provider,
        _secret: &SecretString,
    ) -> Result<ScopeCheck, CredentialError> {
        match self {
            Self::Unconfigured => Err(CredentialError::Store(UNCONFIGURED.to_string())),
            Self::Fixed(fixed) => match &fixed.verdict {
                FixedVerdict::Accept => Ok(ScopeCheck {
                    checked_at: Utc::now(),
                }),
                FixedVerdict::Invalid => Err(CredentialError::Invalid),
                FixedVerdict::Missing(missing) => Err(CredentialError::MissingPermissions {
                    missing: missing.clone(),
                }),
                FixedVerdict::Excess(extra) => Err(CredentialError::ExcessPermissions {
                    extra: extra.clone(),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn fixed(verdict: FixedVerdict) -> CloudProviders {
        CloudProviders::Fixed(FixedCloudProvider {
            offers: ProviderOffers::default(),
            verdict,
        })
    }

    #[tokio::test]
    async fn an_installation_without_a_provider_refuses_to_read_a_catalog() {
        let result = CloudProviders::Unconfigured
            .offers(
                Provider::Scaleway,
                &CloudCredentialId(Uuid::new_v4()),
                &Region::new("fr-par"),
            )
            .await;

        assert!(matches!(result, Err(CatalogError::Unavailable(_))));
    }

    #[tokio::test]
    async fn an_installation_without_a_provider_refuses_to_verify() {
        let result = CloudProviders::Unconfigured
            .verify(Provider::Scaleway, &SecretString::new("key"))
            .await;

        assert!(matches!(result, Err(CredentialError::Store(_))));
    }

    #[tokio::test]
    async fn a_fixed_provider_answers_with_its_verdict() {
        let secret = SecretString::new("key");

        assert!(
            fixed(FixedVerdict::Accept)
                .verify(Provider::Scaleway, &secret)
                .await
                .is_ok()
        );
        assert!(matches!(
            fixed(FixedVerdict::Invalid)
                .verify(Provider::Scaleway, &secret)
                .await,
            Err(CredentialError::Invalid)
        ));
        assert!(matches!(
            fixed(FixedVerdict::Missing(vec!["a".to_string()]))
                .verify(Provider::Scaleway, &secret)
                .await,
            Err(CredentialError::MissingPermissions { .. })
        ));
        assert!(matches!(
            fixed(FixedVerdict::Excess(vec!["b".to_string()]))
                .verify(Provider::Scaleway, &secret)
                .await,
            Err(CredentialError::ExcessPermissions { .. })
        ));
    }
}
