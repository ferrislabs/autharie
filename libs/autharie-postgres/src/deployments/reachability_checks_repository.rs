use chrono::{DateTime, NaiveDate, Utc};

use autharie_domain::{
    CoreError,
    deployments::{
        DeploymentId,
        reachability_history::{DailyCheckCount, ReachabilityCheck, ReachabilityCheckRepository},
    },
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = ReachabilityCheck, backend = Postgres)]
pub struct PostgresReachabilityChecksRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresReachabilityChecksRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl ReachabilityCheckRepository for PostgresReachabilityChecksRepository<'_> {
    async fn record_check(&self, check: ReachabilityCheck) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
                INSERT INTO deployment_reachability_checks (
                    deployment_id,
                    checked_at,
                    reachable
                )
                VALUES ($1, $2, $3)
                "#,
                check.deployment_id.0,
                check.checked_at,
                check.reachable,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to record reachability check: {e}"),
        })?;

        Ok(())
    }

    async fn get_checks_since(
        &self,
        deployment_id: DeploymentId,
        since: DateTime<Utc>,
    ) -> Result<Vec<ReachabilityCheck>, CoreError> {
        let mut tx = self.tx.lock().await;

        let rows: Vec<(DateTime<Utc>, bool)> = sqlx::query_as(
            r#"
            SELECT checked_at, reachable
            FROM deployment_reachability_checks
            WHERE deployment_id = $1
              AND checked_at >= $2
            ORDER BY checked_at ASC, id ASC
            "#,
        )
        .bind(deployment_id.0)
        .bind(since)
        .fetch_all(&mut ***tx)
        .await
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to query checks since {since}: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .map(|(checked_at, reachable)| ReachabilityCheck {
                deployment_id,
                checked_at,
                reachable,
            })
            .collect())
    }

    async fn daily_check_counts(
        &self,
        deployment_id: DeploymentId,
        since: NaiveDate,
    ) -> Result<Vec<DailyCheckCount>, CoreError> {
        let from = since.and_time(chrono::NaiveTime::MIN).and_utc();
        let mut tx = self.tx.lock().await;

        // Days are UTC calendar days whatever the session time zone is, so
        // the console and the database agree on where a day starts.
        let rows: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
            r#"
            SELECT (checked_at AT TIME ZONE 'UTC')::date AS day,
                   COUNT(*) AS checks,
                   COUNT(*) FILTER (WHERE NOT reachable) AS failed
            FROM deployment_reachability_checks
            WHERE deployment_id = $1
              AND checked_at >= $2
            GROUP BY 1
            ORDER BY 1 ASC
            "#,
        )
        .bind(deployment_id.0)
        .bind(from)
        .fetch_all(&mut ***tx)
        .await
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to count checks per day since {since}: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .map(|(day, checks, failed)| DailyCheckCount {
                day,
                checks,
                failed,
            })
            .collect())
    }

    async fn purge_old_checks(&self, retention: chrono::Duration) -> Result<u64, CoreError> {
        let cutoff = Utc::now() - retention;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
                DELETE FROM deployment_reachability_checks
                WHERE checked_at < $1
                "#,
                cutoff,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(|e| CoreError::DatabaseError {
            message: format!("Failed to purge old checks: {e}"),
        })?;

        Ok(result.rows_affected())
    }
}
