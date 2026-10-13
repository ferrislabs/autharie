use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    cells::{
        IssueRecord, SealedAdministrator, Taken, TenantAdministrator, TenantAdministratorRepository,
    },
    dataplane::credential_repository::SealedSecret,
    deployments::DeploymentId,
};
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

struct TakenRow {
    username: Option<String>,
    ciphertext: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
    wrapped_dek: Option<String>,
    key_provider: Option<String>,
    key_name: Option<String>,
    key_version: Option<i32>,
    issued_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    revealed: bool,
    current: bool,
}

impl TakenRow {
    fn into_taken(self, deployment: DeploymentId, now: DateTime<Utc>) -> Result<Taken, CoreError> {
        let (true, Some(issued_at), Some(expires_at), Some(username)) =
            (self.current, self.issued_at, self.expires_at, self.username)
        else {
            return Ok(Taken::Absent);
        };
        if self.revealed {
            return Ok(Taken::Revealed);
        }
        if expires_at <= now {
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
            username,
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
            issued_at,
            expires_at,
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
    async fn lock(&self, deployment: DeploymentId) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtext('tenant-administrator:' || $1::uuid::text)) AS locked",
            deployment.0,
        )
        .fetch_one(&mut ***tx)
        .await
        .map_err(database_error(
            "serialise the administrator issues of a realm",
        ))?;

        Ok(())
    }

    async fn issue_record(
        &self,
        deployment: DeploymentId,
    ) -> Result<Option<IssueRecord>, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query!(
            r#"
            SELECT requested_at, issued_at, expires_at, revealed_at IS NOT NULL AS "revealed!",
                   issued_request_id IS NOT DISTINCT FROM request_id AS "current!"
            FROM tenant_administrators
            WHERE deployment_id = $1
            FOR UPDATE
            "#,
            deployment.0,
        )
        .fetch_optional(&mut ***tx)
        .await
        .map_err(database_error("read the administrator issue of a realm"))?;

        Ok(row.map(|row| IssueRecord {
            requested_at: row.requested_at,
            current: row.current,
            issued_at: row.issued_at,
            expires_at: row.expires_at,
            revealed: row.revealed,
        }))
    }

    async fn request_issue(
        &self,
        deployment: DeploymentId,
        request: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO tenant_administrators (deployment_id, request_id, requested_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (deployment_id) DO UPDATE
            SET request_id = EXCLUDED.request_id, requested_at = EXCLUDED.requested_at
            "#,
            deployment.0,
            request,
            now,
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error("ask for a realm administrator"))?;

        Ok(())
    }

    async fn store_issued(
        &self,
        deployment: DeploymentId,
        request: Uuid,
        administrator: &SealedAdministrator,
    ) -> Result<bool, CoreError> {
        let key_version =
            i32::try_from(administrator.sealed.key.version.value()).map_err(|_| {
                CoreError::InternalError("a key version does not fit the column".to_string())
            })?;

        let mut tx = self.tx.lock().await;
        let stored = sqlx::query!(
            r#"
            UPDATE tenant_administrators
            SET username = $2, ciphertext = $3, nonce = $4, wrapped_dek = $5,
                key_provider = $6, key_name = $7, key_version = $8,
                issued_at = $9, expires_at = $10, revealed_at = NULL, issued_request_id = $11
            WHERE deployment_id = $1 AND request_id = $11
              AND issued_request_id IS DISTINCT FROM $11
            "#,
            deployment.0,
            administrator.username,
            administrator.sealed.ciphertext,
            administrator.sealed.nonce,
            administrator.sealed.wrapped_dek.as_str(),
            administrator.sealed.key.provider.as_str(),
            administrator.sealed.key.name.as_str(),
            key_version,
            administrator.issued_at,
            administrator.expires_at,
            request,
        )
        .execute(&mut ***tx)
        .await
        .map_err(database_error(
            "keep the administrator credential of a realm",
        ))?
        .rows_affected();

        Ok(stored > 0)
    }

    async fn take(&self, deployment: DeploymentId, now: DateTime<Utc>) -> Result<Taken, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query_as!(
            TakenRow,
            r#"
            WITH taken AS (
                SELECT deployment_id, username, ciphertext, nonce, wrapped_dek,
                       key_provider, key_name, key_version, issued_at, expires_at,
                       revealed_at IS NOT NULL AS revealed,
                       issued_request_id IS NOT DISTINCT FROM request_id AS current
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
                  AND t.issued_at IS NOT NULL
                  AND taken.current
            )
            SELECT username, ciphertext, nonce, wrapped_dek, key_provider, key_name, key_version,
                   issued_at, expires_at,
                   revealed AS "revealed!",
                   current AS "current!"
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
            SELECT username, requested_at, issued_at, expires_at,
                   revealed_at IS NOT NULL AS "revealed!",
                   issued_request_id IS NOT DISTINCT FROM request_id AS "current!"
            FROM tenant_administrators
            WHERE deployment_id = $1
            "#,
            deployment.0,
        )
        .fetch_optional(&mut ***tx)
        .await
        .map_err(database_error("describe the administrator of a realm"))?;

        Ok(row.map(|row| {
            IssueRecord {
                requested_at: row.requested_at,
                current: row.current,
                issued_at: row.issued_at,
                expires_at: row.expires_at,
                revealed: row.revealed,
            }
            .describe(row.username, now)
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
