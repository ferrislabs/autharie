use autharie_domain::{
    CoreError,
    billing::{
        BillingSuspension, BillingSuspensionId, StatementId, ports::BillingSuspensionRepository,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::columns::failed;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = BillingSuspension, backend = Postgres)]
pub struct PostgresBillingSuspensionRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresBillingSuspensionRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

struct SuspensionRow {
    id: Uuid,
    organisation_id: Uuid,
    statement_id: Uuid,
    stopped_deployments: Vec<Uuid>,
    suspended_organisation: bool,
    suspended_at: DateTime<Utc>,
    lifted_at: Option<DateTime<Utc>>,
}

impl From<SuspensionRow> for BillingSuspension {
    fn from(row: SuspensionRow) -> Self {
        Self {
            id: BillingSuspensionId(row.id),
            organisation_id: OrganisationId(row.organisation_id),
            statement_id: StatementId(row.statement_id),
            stopped: row
                .stopped_deployments
                .into_iter()
                .map(DeploymentId)
                .collect(),
            suspended_organisation: row.suspended_organisation,
            suspended_at: row.suspended_at,
            lifted_at: row.lifted_at,
        }
    }
}

impl BillingSuspensionRepository for PostgresBillingSuspensionRepository<'_> {
    async fn insert(&self, suspension: &BillingSuspension) -> Result<(), CoreError> {
        let stopped: Vec<Uuid> = suspension.stopped.iter().map(|id| id.0).collect();

        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO billing_suspensions
                (id, organisation_id, statement_id, stopped_deployments, suspended_organisation,
                 suspended_at, lifted_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            suspension.id.0,
            suspension.organisation_id.0,
            suspension.statement_id.0,
            &stopped,
            suspension.suspended_organisation,
            suspension.suspended_at,
            suspension.lifted_at
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("insert the billing suspension"))?;

        Ok(())
    }

    async fn find_active(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Option<BillingSuspension>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SuspensionRow,
                r#"
                SELECT id, organisation_id, statement_id, stopped_deployments,
                       suspended_organisation, suspended_at, lifted_at
                FROM billing_suspensions
                WHERE organisation_id = $1 AND lifted_at IS NULL
                "#,
                organisation_id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the active billing suspension"))?;

        Ok(row.map(BillingSuspension::from))
    }

    async fn lift(&self, id: &BillingSuspensionId, at: DateTime<Utc>) -> Result<(), CoreError> {
        let mut tx = self.tx.lock().await;
        sqlx::query!(
            "UPDATE billing_suspensions SET lifted_at = $2 WHERE id = $1 AND lifted_at IS NULL",
            id.0,
            at
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("lift the billing suspension"))?;

        Ok(())
    }
}
