use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    cells::{
        SealedAdministrator, Taken, TenantAdministrator, TenantAdministratorRepository,
        credential_state,
    },
    dataplane::credential_repository::SealedSecret,
    deployments::DeploymentId,
};
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};

struct TakenRow {
    username: String,
    ciphertext: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
    wrapped_dek: Option<String>,
    key_provider: Option<String>,
    key_name: Option<String>,
    key_version: Option<i32>,
    expires_at: DateTime<Utc>,
    revealed: bool,
}

impl TakenRow {
    fn into_taken(self, deployment: DeploymentId, now: DateTime<Utc>) -> Result<Taken, CoreError> {
        if self.revealed {
            return Ok(Taken::Revealed);
        }
        if self.expires_at <= now {
            return Ok(Taken::Expired);
        }

        let unreadable = || {
            CoreError::InternalError(format!(
                "the administrator credential of deployment {deployment} is incomplete"
            ))
        };
        let (
            Some(ciphertext),
            Some(nonce),
            Some(wrapped_dek),
            Some(key_provider),
            Some(key_name),
            Some(key_version),
        ) = (
            self.ciphertext,
            self.nonce,
            self.wrapped_dek,
            self.key_provider,
            self.key_name,
            self.key_version,
        )
        else {
            return Err(unreadable());
        };
        let version = u32::try_from(key_version).map_err(|_| unreadable())?;

        Ok(Taken::Sealed(SealedAdministrator {
            username: self.username,
            sealed: SealedSecret {
                ciphertext,
                nonce,
                wrapped_dek: WrappedDek::new(wrapped_dek),
                key: KeyRef::new(
                    ProviderName::new(key_provider),
                    KeyName::new(key_name)?,
                    KeyVersion::new(version),
                ),
            },
            expires_at: self.expires_at,
        }))
    }
}

fn database_error(action: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {action}: {error}"),
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub struct PostgresTenantAdministratorRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresTenantAdministratorRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl TenantAdministratorRepository for PostgresTenantAdministratorRepository<'_> {
    async fn insert_if_absent(
        &self,
        deployment: DeploymentId,
        administrator: &SealedAdministrator,
    ) -> Result<bool, CoreError> {
        let key_version =
            i32::try_from(administrator.sealed.key.version.value()).map_err(|_| {
                CoreError::InternalError("a key version does not fit the column".to_string())
            })?;

        let mut tx = self.tx.lock().await;
        let inserted = sqlx::query!(
            r#"
            INSERT INTO tenant_administrators (
                deployment_id, username, ciphertext, nonce, wrapped_dek,
                key_provider, key_name, key_version, expires_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (deployment_id) DO NOTHING
            "#,
            deployment.0,
            administrator.username,
            administrator.sealed.ciphertext,
            administrator.sealed.nonce,
            administrator.sealed.wrapped_dek.as_str(),
            administrator.sealed.key.provider.as_str(),
            administrator.sealed.key.name.as_str(),
            key_version,
            administrator.expires_at,
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error(
            "keep the administrator credential of a realm",
        ))?
        .rows_affected();

        Ok(inserted > 0)
    }

    async fn take(&self, deployment: DeploymentId, now: DateTime<Utc>) -> Result<Taken, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query_as!(
            TakenRow,
            r#"
            WITH taken AS (
                SELECT deployment_id, username, ciphertext, nonce, wrapped_dek,
                       key_provider, key_name, key_version, expires_at,
                       revealed_at IS NOT NULL AS revealed
                FROM tenant_administrators
                WHERE deployment_id = $1
                FOR UPDATE
            ), wiped AS (
                UPDATE tenant_administrators t
                SET ciphertext = NULL, nonce = NULL, wrapped_dek = NULL,
                    key_provider = NULL, key_name = NULL, key_version = NULL,
                    revealed_at = CASE WHEN taken.expires_at > $2 THEN $2 ELSE t.revealed_at END
                FROM taken
                WHERE t.deployment_id = taken.deployment_id
                  AND t.revealed_at IS NULL
            )
            SELECT username AS "username!",
                   ciphertext, nonce, wrapped_dek, key_provider, key_name, key_version,
                   expires_at AS "expires_at!",
                   revealed AS "revealed!"
            FROM taken
            "#,
            deployment.0,
            now,
        )
        .fetch_optional(&mut ***tx)
        .await
        .map_err(database_error(
            "take the administrator credential of a realm",
        ))?;

        match row {
            None => Ok(Taken::Absent),
            Some(row) => row.into_taken(deployment, now),
        }
    }

    async fn describe(
        &self,
        deployment: DeploymentId,
        now: DateTime<Utc>,
    ) -> Result<Option<TenantAdministrator>, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query!(
            r#"
            SELECT username, expires_at, revealed_at IS NOT NULL AS "revealed!"
            FROM tenant_administrators
            WHERE deployment_id = $1
            "#,
            deployment.0,
        )
        .fetch_optional(&mut ***tx)
        .await
        .map_err(database_error("describe the administrator of a realm"))?;

        Ok(row.map(|row| TenantAdministrator {
            username: row.username,
            credential: credential_state(row.revealed, row.expires_at, now),
            expires_at: row.expires_at,
        }))
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64, CoreError> {
        let mut tx = self.tx.lock().await;
        let purged = sqlx::query!(
            r#"
            UPDATE tenant_administrators
            SET ciphertext = NULL, nonce = NULL, wrapped_dek = NULL,
                key_provider = NULL, key_name = NULL, key_version = NULL
            WHERE expires_at <= $1 AND ciphertext IS NOT NULL
            "#,
            now,
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("wipe expired administrator credentials"))?
        .rows_affected();

        Ok(purged)
    }
}
