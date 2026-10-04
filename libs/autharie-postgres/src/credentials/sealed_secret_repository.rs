use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    dataplane::{
        credential::CloudCredentialId,
        credential_repository::{SealedCredential, SealedSecret, SealedSecretRepository},
    },
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use uuid::Uuid;

use super::parse_provider;

struct SealedRow {
    id: Uuid,
    organisation_id: Uuid,
    provider: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    wrapped_dek: String,
    key_provider: String,
    key_name: String,
    key_version: i32,
}

impl SealedRow {
    fn into_sealed(self) -> Result<SealedCredential, CoreError> {
        let version = u32::try_from(self.key_version).map_err(|_| {
            CoreError::InternalError(format!(
                "credential {} records a negative key version",
                self.id
            ))
        })?;

        Ok(SealedCredential {
            id: CloudCredentialId(self.id),
            organisation_id: OrganisationId(self.organisation_id),
            provider: parse_provider(&self.provider)?,
            sealed: SealedSecret {
                ciphertext: self.ciphertext,
                nonce: self.nonce,
                wrapped_dek: WrappedDek::new(self.wrapped_dek),
                key: KeyRef::new(
                    ProviderName::new(self.key_provider),
                    KeyName::new(self.key_name)?,
                    KeyVersion::new(version),
                ),
            },
        })
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = SealedSecret, backend = Postgres)]
pub struct PostgresSealedSecretRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresSealedSecretRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl SealedSecretRepository for PostgresSealedSecretRepository<'_> {
    async fn insert(&self, credential: &SealedCredential) -> Result<(), CoreError> {
        let key_version = i32::try_from(credential.sealed.key.version.value()).map_err(|_| {
            CoreError::InternalError("a key version does not fit the column".to_string())
        })?;

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO cloud_credential_secrets (
                id,
                organisation_id,
                provider,
                ciphertext,
                nonce,
                wrapped_dek,
                key_provider,
                key_name,
                key_version
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
                credential.id.0,
                credential.organisation_id.0,
                credential.provider.as_str(),
                credential.sealed.ciphertext,
                credential.sealed.nonce,
                credential.sealed.wrapped_dek.as_str(),
                credential.sealed.key.provider.as_str(),
                credential.sealed.key.name.as_str(),
                key_version,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to store a sealed credential: {e}"),
        })?;

        Ok(())
    }

    async fn get(&self, id: &CloudCredentialId) -> Result<Option<SealedCredential>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SealedRow,
                r#"
            SELECT id,
                   organisation_id,
                   provider,
                   ciphertext,
                   nonce,
                   wrapped_dek,
                   key_provider,
                   key_name,
                   key_version
            FROM cloud_credential_secrets
            WHERE id = $1
            "#,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to read a sealed credential: {e}"),
        })?;

        row.map(SealedRow::into_sealed).transpose()
    }

    async fn delete(&self, id: &CloudCredentialId) -> Result<bool, CoreError> {
        let affected = {
            let mut tx = self.tx.lock().await;
            sqlx::query!("DELETE FROM cloud_credential_secrets WHERE id = $1", id.0)
                .execute(&mut ***tx)
                .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to delete a sealed credential: {e}"),
        })?
        .rows_affected();

        Ok(affected > 0)
    }
}
