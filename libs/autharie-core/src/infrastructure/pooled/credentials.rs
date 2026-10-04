use std::sync::Arc;

use autharie_domain::{
    dataplane::{
        cloud_provider::Provider,
        credential::{CloudCredentialId, CloudCredentialStore, CredentialError, SecretString},
    },
    organisation::OrganisationId,
};
use autharie_persistence::with_tx;
use autharie_postgres::credentials::PostgresSealedSecretRepository;
use autharie_transit::TransitKeyProvider;
use sqlx::PgPool;

use crate::infrastructure::credentials::EnvelopeCredentialStore;

#[derive(Clone)]
pub struct PooledCredentialStore {
    pool: PgPool,
    keys: Arc<TransitKeyProvider>,
}

impl PooledCredentialStore {
    pub fn new(pool: PgPool, keys: Arc<TransitKeyProvider>) -> Self {
        Self { pool, keys }
    }
}

fn store_failure(error: sqlx::Error) -> CredentialError {
    CredentialError::Store(error.to_string())
}

impl CloudCredentialStore for PooledCredentialStore {
    async fn put(
        &self,
        organisation_id: OrganisationId,
        provider: Provider,
        secret: SecretString,
    ) -> Result<CloudCredentialId, CredentialError> {
        with_tx(&self.pool, store_failure, async |tx| {
            EnvelopeCredentialStore::new(PostgresSealedSecretRepository::new(&tx), &*self.keys)?
                .put(organisation_id, provider, secret)
                .await
        })
        .await
    }

    async fn get_for_provisioning(
        &self,
        id: &CloudCredentialId,
    ) -> Result<SecretString, CredentialError> {
        with_tx(&self.pool, store_failure, async |tx| {
            EnvelopeCredentialStore::new(PostgresSealedSecretRepository::new(&tx), &*self.keys)?
                .get_for_provisioning(id)
                .await
        })
        .await
    }

    async fn delete(&self, id: &CloudCredentialId) -> Result<(), CredentialError> {
        with_tx(&self.pool, store_failure, async |tx| {
            EnvelopeCredentialStore::new(PostgresSealedSecretRepository::new(&tx), &*self.keys)?
                .delete(id)
                .await
        })
        .await
    }
}
