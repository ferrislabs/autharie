use autharie_domain::{
    CoreError,
    dataplane::{
        credential::{CloudCredential, CloudCredentialId, ScopeCheck},
        credential_repository::CloudCredentialRepository,
    },
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::parse_provider;

struct CredentialRow {
    id: Uuid,
    organisation_id: Uuid,
    provider: String,
    label: String,
    scope_checked_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl CredentialRow {
    fn into_credential(self) -> Result<CloudCredential, CoreError> {
        Ok(CloudCredential {
            id: CloudCredentialId(self.id),
            organisation_id: OrganisationId(self.organisation_id),
            provider: parse_provider(&self.provider)?,
            label: self.label,
            scope_check: ScopeCheck {
                checked_at: self.scope_checked_at,
            },
            created_at: self.created_at,
        })
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = CloudCredential, backend = Postgres)]
pub struct PostgresCloudCredentialRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresCloudCredentialRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl CloudCredentialRepository for PostgresCloudCredentialRepository<'_> {
    async fn insert(&self, credential: &CloudCredential) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO cloud_credentials (
                id,
                organisation_id,
                provider,
                label,
                scope_checked_at,
                created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
                credential.id.0,
                credential.organisation_id.0,
                credential.provider.as_str(),
                credential.label,
                credential.scope_check.checked_at,
                credential.created_at,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to record a cloud credential: {e}"),
        })?;

        Ok(())
    }

    async fn get(&self, id: &CloudCredentialId) -> Result<Option<CloudCredential>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                CredentialRow,
                r#"
            SELECT id, organisation_id, provider, label, scope_checked_at, created_at
            FROM cloud_credentials
            WHERE id = $1
            "#,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to read a cloud credential: {e}"),
        })?;

        row.map(CredentialRow::into_credential).transpose()
    }

    async fn list_for_organisation(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Vec<CloudCredential>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                CredentialRow,
                r#"
            SELECT id, organisation_id, provider, label, scope_checked_at, created_at
            FROM cloud_credentials
            WHERE organisation_id = $1
            ORDER BY created_at ASC, id ASC
            "#,
                organisation_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to list cloud credentials: {e}"),
        })?;

        rows.into_iter()
            .map(CredentialRow::into_credential)
            .collect()
    }

    async fn is_in_use(&self, id: &CloudCredentialId) -> Result<bool, CoreError> {
        let in_use = {
            let mut tx = self.tx.lock().await;
            sqlx::query_scalar!(
                r#"
            SELECT (
                EXISTS (
                    SELECT 1
                    FROM deployments
                    WHERE credential_id = $1
                      AND status <> 'deleted'
                )
                OR EXISTS (
                    SELECT 1
                    FROM data_planes dp
                    JOIN cluster_inventory ci ON ci.data_plane_id = dp.id
                    WHERE dp.credential_id = $1
                      AND ci.released_at IS NULL
                )
            ) AS "in_use!"
            "#,
                id.0
            )
            .fetch_one(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to check whether a credential is in use: {e}"),
        })?;

        Ok(in_use)
    }
}
