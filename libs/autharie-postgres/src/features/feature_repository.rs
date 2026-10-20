use std::collections::HashMap;

use autharie_domain::{
    CoreError,
    features::{
        Feature, OverrideTally,
        ports::{FeatureRepository, FlagChange},
    },
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = FeatureFlag, backend = Postgres)]
pub struct PostgresFeatureRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresFeatureRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

fn known(key: &str) -> Option<Feature> {
    key.parse().ok()
}

impl FeatureRepository for PostgresFeatureRepository<'_> {
    async fn defaults(&self) -> Result<HashMap<Feature, bool>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!("SELECT key, enabled_by_default FROM feature_flags")
                .fetch_all(&mut ***tx)
                .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to read the feature defaults: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|row| known(&row.key).map(|feature| (feature, row.enabled_by_default)))
            .collect())
    }

    async fn set_default(
        &self,
        feature: Feature,
        enabled: bool,
        change: &FlagChange,
    ) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO feature_flags (key, enabled_by_default, updated_at, updated_by)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (key) DO UPDATE
                SET enabled_by_default = EXCLUDED.enabled_by_default,
                    updated_at = EXCLUDED.updated_at,
                    updated_by = EXCLUDED.updated_by
            "#,
            feature.as_str(),
            enabled,
            change.at,
            change.by
        )
        .execute(&mut ***tx)
        .await
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to set the feature default: {e}"),
        })?;

        Ok(())
    }

    async fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<HashMap<Feature, bool>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "SELECT key, enabled FROM organisation_feature_flags WHERE organisation_id = $1",
                organisation_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to read the feature overrides: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|row| known(&row.key).map(|feature| (feature, row.enabled)))
            .collect())
    }

    async fn set_override(
        &self,
        organisation_id: OrganisationId,
        feature: Feature,
        enabled: Option<bool>,
        change: &FlagChange,
    ) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        match enabled {
            Some(enabled) => {
                sqlx::query!(
                    r#"
                    INSERT INTO organisation_feature_flags
                        (organisation_id, key, enabled, updated_at, updated_by)
                    VALUES ($1, $2, $3, $4, $5)
                    ON CONFLICT (organisation_id, key) DO UPDATE
                        SET enabled = EXCLUDED.enabled,
                            updated_at = EXCLUDED.updated_at,
                            updated_by = EXCLUDED.updated_by
                    "#,
                    organisation_id.0,
                    feature.as_str(),
                    enabled,
                    change.at,
                    change.by
                )
                .execute(&mut ***tx)
                .await
            }
            None => sqlx::query!(
                "DELETE FROM organisation_feature_flags WHERE organisation_id = $1 AND key = $2",
                organisation_id.0,
                feature.as_str()
            )
            .execute(&mut ***tx)
            .await,
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to set the feature override: {e}"),
        })?;

        Ok(())
    }

    async fn override_tallies(&self) -> Result<HashMap<Feature, OverrideTally>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
                SELECT key,
                       COUNT(*) FILTER (WHERE enabled) AS "on!",
                       COUNT(*) FILTER (WHERE NOT enabled) AS "off!"
                FROM organisation_feature_flags
                GROUP BY key
                "#
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to count the feature overrides: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                known(&row.key).map(|feature| {
                    (
                        feature,
                        OverrideTally {
                            on: row.on as usize,
                            off: row.off as usize,
                        },
                    )
                })
            })
            .collect())
    }
}
