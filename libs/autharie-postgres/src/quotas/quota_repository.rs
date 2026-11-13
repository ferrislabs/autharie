use std::collections::HashMap;

use autharie_domain::{
    CoreError,
    organisation::OrganisationId,
    quotas::{
        Quota, QuotaKind,
        ports::{QuotaChange, QuotaRepository},
    },
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Quota, backend = Postgres)]
pub struct PostgresQuotaRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresQuotaRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

impl QuotaRepository for PostgresQuotaRepository<'_> {
    async fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<HashMap<QuotaKind, Quota>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "SELECT kind, max_count FROM organisation_quotas WHERE organisation_id = $1",
                organisation_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to read the quota overrides: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let kind: QuotaKind = row.kind.parse().ok()?;
                let quota = match row.max_count {
                    Some(max) => Quota::Limited(u32::try_from(max).ok()?),
                    None => Quota::Unlimited,
                };

                Some((kind, quota))
            })
            .collect())
    }

    async fn set_override(
        &self,
        organisation_id: OrganisationId,
        kind: QuotaKind,
        quota: Option<Quota>,
        change: &QuotaChange,
    ) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        match quota {
            Some(quota) => {
                let max = match quota {
                    Quota::Limited(max) => {
                        Some(i32::try_from(max).map_err(|_| CoreError::DatabaseError {
                            message: format!("a quota of {max} does not fit the column"),
                        })?)
                    }
                    Quota::Unlimited => None,
                };

                sqlx::query!(
                    r#"
                    INSERT INTO organisation_quotas
                        (organisation_id, kind, max_count, updated_at, updated_by)
                    VALUES ($1, $2, $3, $4, $5)
                    ON CONFLICT (organisation_id, kind) DO UPDATE
                        SET max_count = EXCLUDED.max_count,
                            updated_at = EXCLUDED.updated_at,
                            updated_by = EXCLUDED.updated_by
                    "#,
                    organisation_id.0,
                    kind.as_str(),
                    max,
                    change.at,
                    change.by
                )
                .execute(&mut ***tx)
                .await
            }
            None => {
                sqlx::query!(
                    "DELETE FROM organisation_quotas WHERE organisation_id = $1 AND kind = $2",
                    organisation_id.0,
                    kind.as_str()
                )
                .execute(&mut ***tx)
                .await
            }
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to set the quota override: {e}"),
        })?;

        Ok(())
    }
}
