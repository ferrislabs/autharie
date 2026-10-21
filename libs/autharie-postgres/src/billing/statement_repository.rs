use std::collections::HashMap;

use autharie_domain::{
    CoreError,
    billing::{
        BillingPeriod, Money, ProviderInvoice, ProviderInvoiceId, Statement, StatementId,
        StatementLine, StatementParts, StatementStatus, SubscriptionId, TierCode,
        ports::StatementRepository,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::columns::{failed, invalid, parsed};

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Statement, backend = Postgres)]
pub struct PostgresStatementRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresStatementRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

struct StatementRow {
    id: Uuid,
    organisation_id: Uuid,
    period: String,
    status: String,
    provider_invoice_id: Option<String>,
    hosted_invoice_url: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

struct LineRow {
    statement_id: Uuid,
    deployment_id: Uuid,
    subscription_id: Uuid,
    tier_code: String,
    monthly_price_minor: i64,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    covered_seconds: i64,
    amount_minor: i64,
}

impl TryFrom<LineRow> for StatementLine {
    type Error = CoreError;

    fn try_from(row: LineRow) -> Result<Self, Self::Error> {
        Ok(Self {
            deployment_id: DeploymentId(row.deployment_id),
            subscription_id: SubscriptionId(row.subscription_id),
            tier: parsed::<TierCode>("billing_statement_lines.tier_code", &row.tier_code)?,
            monthly_price: Money::from_minor(row.monthly_price_minor),
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            covered_seconds: row.covered_seconds,
            amount: Money::from_minor(row.amount_minor),
        })
    }
}

fn statement(row: StatementRow, lines: Vec<StatementLine>) -> Result<Statement, CoreError> {
    let invoice = match (row.provider_invoice_id, row.hosted_invoice_url) {
        (Some(id), Some(hosted_url)) => Some(ProviderInvoice {
            id: ProviderInvoiceId::parse(&id)
                .map_err(|reason| invalid("billing_statements.provider_invoice_id", &id, reason))?,
            hosted_url,
        }),
        (None, None) => None,
        (Some(id), None) => {
            return Err(invalid(
                "billing_statements.hosted_invoice_url",
                &id,
                "the statement has an invoice but no hosted url",
            ));
        }
        (None, Some(hosted_url)) => {
            return Err(invalid(
                "billing_statements.provider_invoice_id",
                &hosted_url,
                "the statement has a hosted url but no invoice",
            ));
        }
    };

    Ok(Statement::from(StatementParts {
        id: StatementId(row.id),
        organisation_id: OrganisationId(row.organisation_id),
        period: parsed::<BillingPeriod>("billing_statements.period", &row.period)?,
        lines,
        status: parsed::<StatementStatus>("billing_statements.status", &row.status)?,
        invoice,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

impl PostgresStatementRepository<'_> {
    async fn assemble(&self, rows: Vec<StatementRow>) -> Result<Vec<Statement>, CoreError> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();

        let line_rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                LineRow,
                r#"
                SELECT statement_id, deployment_id, subscription_id, tier_code,
                       monthly_price_minor, starts_at, ends_at, covered_seconds, amount_minor
                FROM billing_statement_lines
                WHERE statement_id = ANY($1)
                ORDER BY statement_id, position
                "#,
                &ids
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("load the statement lines"))?;

        let mut lines: HashMap<Uuid, Vec<StatementLine>> = HashMap::new();
        for line_row in line_rows {
            let statement_id = line_row.statement_id;
            lines
                .entry(statement_id)
                .or_default()
                .push(StatementLine::try_from(line_row)?);
        }

        rows.into_iter()
            .map(|row| {
                let own_lines = lines.remove(&row.id).unwrap_or_default();
                statement(row, own_lines)
            })
            .collect()
    }

    async fn assemble_one(
        &self,
        row: Option<StatementRow>,
    ) -> Result<Option<Statement>, CoreError> {
        match row {
            Some(row) => Ok(self.assemble(vec![row]).await?.into_iter().next()),
            None => Ok(None),
        }
    }
}

impl StatementRepository for PostgresStatementRepository<'_> {
    async fn insert(&self, statement: &Statement) -> Result<bool, CoreError> {
        let period = statement.period().to_string();
        let status = statement.status().to_string();
        let invoice = statement.invoice();

        let mut tx = self.tx.lock().await;
        let inserted = sqlx::query!(
            r#"
            INSERT INTO billing_statements
                (id, organisation_id, period, status, total_minor, provider_invoice_id,
                 hosted_invoice_url, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (organisation_id, period) DO NOTHING
            "#,
            statement.id().0,
            statement.organisation_id().0,
            period,
            status,
            statement.total().minor(),
            invoice.map(|invoice| invoice.id.as_str()),
            invoice.map(|invoice| invoice.hosted_url.as_str()),
            statement.created_at(),
            statement.updated_at()
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("insert the statement"))?;

        if inserted.rows_affected() == 0 {
            return Ok(false);
        }

        for (index, line) in statement.lines().iter().enumerate() {
            let position = i32::try_from(index).map_err(|reason| {
                invalid(
                    "billing_statement_lines.position",
                    &index.to_string(),
                    reason,
                )
            })?;
            let tier = line.tier.as_str();

            sqlx::query!(
                r#"
                INSERT INTO billing_statement_lines
                    (statement_id, position, deployment_id, subscription_id, tier_code,
                     monthly_price_minor, starts_at, ends_at, covered_seconds, amount_minor)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
                statement.id().0,
                position,
                line.deployment_id.0,
                line.subscription_id.0,
                tier,
                line.monthly_price.minor(),
                line.starts_at,
                line.ends_at,
                line.covered_seconds,
                line.amount.minor()
            )
            .execute(&mut ***tx)
            .await
            .map_err(failed("insert the statement line"))?;
        }

        Ok(true)
    }

    async fn update(&self, statement: &Statement) -> Result<(), CoreError> {
        let status = statement.status().to_string();
        let invoice = statement.invoice();

        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            UPDATE billing_statements
            SET status = $2,
                provider_invoice_id = $3,
                hosted_invoice_url = $4,
                updated_at = $5
            WHERE id = $1
            "#,
            statement.id().0,
            status,
            invoice.map(|invoice| invoice.id.as_str()),
            invoice.map(|invoice| invoice.hosted_url.as_str()),
            statement.updated_at()
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("update the statement"))?;

        Ok(())
    }

    async fn find(&self, id: &StatementId) -> Result<Option<Statement>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                StatementRow,
                r#"
                SELECT id, organisation_id, period, status, provider_invoice_id,
                       hosted_invoice_url, created_at, updated_at
                FROM billing_statements
                WHERE id = $1
                FOR UPDATE
                "#,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the statement"))?;

        self.assemble_one(row).await
    }

    async fn find_by_period(
        &self,
        organisation_id: &OrganisationId,
        period: BillingPeriod,
    ) -> Result<Option<Statement>, CoreError> {
        let period = period.to_string();

        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                StatementRow,
                r#"
                SELECT id, organisation_id, period, status, provider_invoice_id,
                       hosted_invoice_url, created_at, updated_at
                FROM billing_statements
                WHERE organisation_id = $1 AND period = $2
                "#,
                organisation_id.0,
                period
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the statement of the period"))?;

        self.assemble_one(row).await
    }

    async fn find_by_invoice(
        &self,
        invoice: &ProviderInvoiceId,
    ) -> Result<Option<Statement>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                StatementRow,
                r#"
                SELECT id, organisation_id, period, status, provider_invoice_id,
                       hosted_invoice_url, created_at, updated_at
                FROM billing_statements
                WHERE provider_invoice_id = $1
                FOR UPDATE
                "#,
                invoice.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the statement of the invoice"))?;

        self.assemble_one(row).await
    }

    async fn list_for(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Vec<Statement>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                StatementRow,
                r#"
                SELECT id, organisation_id, period, status, provider_invoice_id,
                       hosted_invoice_url, created_at, updated_at
                FROM billing_statements
                WHERE organisation_id = $1
                ORDER BY period DESC, created_at DESC, id DESC
                "#,
                organisation_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("list the statements"))?;

        self.assemble(rows).await
    }

    async fn list_unsent(&self) -> Result<Vec<Statement>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                StatementRow,
                r#"
                SELECT id, organisation_id, period, status, provider_invoice_id,
                       hosted_invoice_url, created_at, updated_at
                FROM billing_statements
                WHERE status IN ('held', 'draft')
                ORDER BY period, created_at, id
                "#
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("list the unsent statements"))?;

        self.assemble(rows).await
    }
}
