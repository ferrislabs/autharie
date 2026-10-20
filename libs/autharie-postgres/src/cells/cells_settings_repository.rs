use autharie_domain::{
    CoreError,
    cells::{CellsSettings, CellsSettingsRepository},
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

struct SettingsRow {
    organisation_id: Option<Uuid>,
    updated_at: DateTime<Utc>,
    updated_by: Option<String>,
}

impl From<SettingsRow> for CellsSettings {
    fn from(row: SettingsRow) -> Self {
        Self {
            organisation_id: row.organisation_id.map(OrganisationId),
            updated_at: row.updated_at,
            updated_by: row.updated_by,
        }
    }
}

fn database_error(action: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {action}: {error}"),
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = CellsSettings, backend = Postgres)]
pub struct PostgresCellsSettingsRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresCellsSettingsRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl CellsSettingsRepository for PostgresCellsSettingsRepository<'_> {
    async fn get(&self) -> Result<CellsSettings, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query_as!(
            SettingsRow,
            r#"
            SELECT organisation_id, updated_at, updated_by
            FROM platform_cells_settings
            WHERE id
            "#
        )
        .fetch_one(&mut ***tx)
        .await
        .map_err(database_error("read the cells settings"))?;

        Ok(row.into())
    }

    async fn set(
        &self,
        organisation: Option<OrganisationId>,
        updated_by: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<CellsSettings, CoreError> {
        let mut tx = self.tx.lock().await;
        let row = sqlx::query_as!(
            SettingsRow,
            r#"
            UPDATE platform_cells_settings
            SET organisation_id = $1, updated_by = $2, updated_at = $3
            WHERE id
            RETURNING organisation_id, updated_at, updated_by
            "#,
            organisation.map(|id| id.0),
            updated_by,
            updated_at,
        )
        .fetch_one(&mut ***tx)
        .await
        .map_err(database_error("write the cells settings"))?;

        Ok(row.into())
    }
}
