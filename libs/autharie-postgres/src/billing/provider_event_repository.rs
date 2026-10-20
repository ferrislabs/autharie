use autharie_domain::{
    CoreError,
    billing::{ProviderEventId, ports::ProviderEventLog},
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};

use super::columns::failed;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = ProviderEvent, backend = Postgres)]
pub struct PostgresProviderEventRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresProviderEventRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

impl ProviderEventLog for PostgresProviderEventRepository<'_> {
    async fn record(
        &self,
        id: &ProviderEventId,
        received_at: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let recorded = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
                INSERT INTO billing_provider_events (id, received_at)
                VALUES ($1, $2)
                ON CONFLICT (id) DO NOTHING
                "#,
                id.as_str(),
                received_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(failed("record the provider event"))?;

        Ok(recorded.rows_affected() == 1)
    }
}
