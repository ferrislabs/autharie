use autharie_domain::{
    CoreError,
    deployments::DeploymentId,
    iam::{
        Envelope, IamOperationKind, IamRealm, IamRequest, IamState, IamVerdict,
        ports::IamRepository,
    },
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {what}: {error}"),
    }
}

fn corrupt(what: impl std::fmt::Display) -> CoreError {
    CoreError::InternalError(format!("a stored iam request cannot be read: {what}"))
}

struct RequestRow {
    request_id: Uuid,
    deployment_id: Uuid,
    realm: String,
    operation: String,
    actor: String,
    phase: String,
    message: Option<String>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl RequestRow {
    fn into_request(self) -> Result<IamRequest, CoreError> {
        let state = match self.phase.as_str() {
            "pending" => IamState::Pending,
            "done" => IamState::Done,
            "failed" => IamState::Failed {
                message: self.message.unwrap_or_default(),
            },
            "delivered" => IamState::Delivered,
            other => return Err(corrupt(format!("phase '{other}'"))),
        };

        Ok(IamRequest {
            id: self.request_id,
            deployment_id: DeploymentId(self.deployment_id),
            realm: IamRealm::parse(&self.realm).map_err(corrupt)?,
            operation: IamOperationKind::parse(&self.operation)
                .ok_or_else(|| corrupt(format!("operation '{}'", self.operation)))?,
            actor: self.actor,
            state,
            created_at: self.created_at,
            expires_at: self.expires_at,
        })
    }
}

struct VerdictColumns<'a> {
    phase: &'static str,
    envelope: Option<Value>,
    message: Option<&'a str>,
}

fn verdict_columns(verdict: &IamVerdict) -> Result<VerdictColumns<'_>, CoreError> {
    Ok(match verdict {
        IamVerdict::Done(envelope) => VerdictColumns {
            phase: "done",
            envelope: Some(envelope.clone().into_value()?),
            message: None,
        },
        IamVerdict::Failed { message } => VerdictColumns {
            phase: "failed",
            envelope: None,
            message: Some(message),
        },
    })
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Iam, backend = Postgres)]
pub struct PostgresIamRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresIamRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl IamRepository for PostgresIamRepository<'_> {
    async fn open(&self, request: &IamRequest, max_outstanding: usize) -> Result<bool, CoreError> {
        let max_outstanding = i64::try_from(max_outstanding)
            .map_err(|_| CoreError::InternalError("a limit does not fit the column".to_string()))?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))",
                request.deployment_id.0.to_string()
            )
            .execute(&mut ***tx)
            .await
            .map_err(db("lock a deployment's iam requests"))?;

            sqlx::query!(
                r#"
            INSERT INTO iam_requests (
                request_id, deployment_id, realm, operation, actor, phase, created_at, expires_at
            )
            SELECT $1, $2, $3, $4, $5, 'pending', $6, $7
            WHERE (
                SELECT count(*)
                FROM iam_requests
                WHERE deployment_id = $2
                  AND phase <> 'delivered'
                  AND expires_at > $6
            ) < $8
            "#,
                request.id,
                request.deployment_id.0,
                request.realm.as_str(),
                request.operation.as_str(),
                request.actor,
                request.created_at,
                request.expires_at,
                max_outstanding
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store an iam request"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<IamRequest>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                RequestRow,
                r#"
            SELECT request_id, deployment_id, realm, operation, actor, phase, message,
                   created_at, expires_at
            FROM iam_requests
            WHERE request_id = $1 AND deployment_id = $2 AND expires_at > $3
            "#,
                request_id,
                deployment_id.0,
                now
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read an iam request"))?;

        row.map(RequestRow::into_request).transpose()
    }

    async fn record_outcome(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        verdict: &IamVerdict,
        now: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let columns = verdict_columns(verdict)?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE iam_requests
            SET phase = $4, envelope = $5, message = $6
            WHERE request_id = $1
              AND deployment_id = $2
              AND phase = 'pending'
              AND expires_at > $3
            "#,
                request_id,
                deployment_id.0,
                now,
                columns.phase,
                columns.envelope as Option<Value>,
                columns.message
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record an iam outcome"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn take_envelope(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<Envelope>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            WITH old AS (
                SELECT request_id, envelope
                FROM iam_requests
                WHERE request_id = $1
                  AND deployment_id = $2
                  AND phase = 'done'
                  AND expires_at > $3
                FOR UPDATE
            )
            UPDATE iam_requests AS request
            SET phase = 'delivered', envelope = NULL
            FROM old
            WHERE request.request_id = old.request_id
            RETURNING old.envelope AS "envelope!"
            "#,
                request_id,
                deployment_id.0,
                now
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("take a sealed result"))?;

        row.map(|row| Envelope::parse(row.envelope).map_err(corrupt))
            .transpose()
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!("DELETE FROM iam_requests WHERE expires_at <= $1", now)
                .execute(&mut ***tx)
                .await
        }
        .map_err(db("purge expired iam requests"))?;

        Ok(result.rows_affected())
    }
}
