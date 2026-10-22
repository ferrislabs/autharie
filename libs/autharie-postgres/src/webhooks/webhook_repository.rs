use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    dataplane::credential_repository::SealedSecret,
    deployments::DeploymentId,
    webhooks::{
        HeaderName, RequestKind, RequestPhase, SecretRecord, StoredWebhook, Webhook,
        WebhookEndpoint, WebhookEvent, WebhookId, WebhookName, WebhookRealm, WebhookRequest,
        WebhookState, WebhookSync, ports::WebhookRepository,
    },
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {what}: {error}"),
    }
}

fn corrupt(what: impl std::fmt::Display) -> CoreError {
    CoreError::InternalError(format!("a stored webhook cannot be read: {what}"))
}

struct WebhookRow {
    id: Uuid,
    deployment_id: Uuid,
    realm: String,
    name: String,
    description: Option<String>,
    endpoint: String,
    events: Vec<String>,
    header_names: Vec<String>,
    headers_ciphertext: Option<Vec<u8>>,
    headers_nonce: Option<Vec<u8>>,
    headers_wrapped_dek: Option<String>,
    headers_key_provider: Option<String>,
    headers_key_name: Option<String>,
    headers_key_version: Option<i32>,
    state: String,
    ferriskey_id: Option<String>,
    failure_reason: Option<String>,
    desired_revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    secret_version: Option<i64>,
    secret_received_at: Option<DateTime<Utc>>,
}

impl WebhookRow {
    fn into_stored(self) -> Result<StoredWebhook, CoreError> {
        let state = match self.state.as_str() {
            "pending" => WebhookState::Pending,
            "active" => WebhookState::Active {
                ferriskey_id: self
                    .ferriskey_id
                    .ok_or_else(|| corrupt("active without id"))?,
            },
            "failed" => WebhookState::Failed {
                reason: self.failure_reason.unwrap_or_default(),
            },
            other => return Err(corrupt(format!("state '{other}'"))),
        };

        let sealed_headers = match (
            self.headers_ciphertext,
            self.headers_nonce,
            self.headers_wrapped_dek,
            self.headers_key_provider,
            self.headers_key_name,
            self.headers_key_version,
        ) {
            (None, None, None, None, None, None) => None,
            (
                Some(ciphertext),
                Some(nonce),
                Some(wrapped_dek),
                Some(provider),
                Some(name),
                Some(version),
            ) => Some(SealedSecret {
                ciphertext,
                nonce,
                wrapped_dek: WrappedDek::new(wrapped_dek),
                key: KeyRef::new(
                    ProviderName::new(provider),
                    KeyName::new(name)?,
                    KeyVersion::new(u32::try_from(version).map_err(corrupt)?),
                ),
            }),
            _ => return Err(corrupt("half a sealed header set")),
        };

        Ok(StoredWebhook {
            webhook: Webhook {
                id: WebhookId(self.id),
                deployment_id: DeploymentId(self.deployment_id),
                realm: WebhookRealm::parse(&self.realm).map_err(corrupt)?,
                name: WebhookName::parse(&self.name).map_err(corrupt)?,
                description: self.description,
                endpoint: WebhookEndpoint::parse(&self.endpoint).map_err(corrupt)?,
                events: self
                    .events
                    .iter()
                    .map(|event| WebhookEvent::parse(event))
                    .collect::<Result<_, _>>()
                    .map_err(corrupt)?,
                headers: self
                    .header_names
                    .iter()
                    .map(|name| HeaderName::parse(name))
                    .collect::<Result<_, _>>()
                    .map_err(corrupt)?,
                state,
                secret_version: self
                    .secret_version
                    .map(u64::try_from)
                    .transpose()
                    .map_err(corrupt)?,
                secret_received_at: self.secret_received_at,
                desired_revision: u64::try_from(self.desired_revision).map_err(corrupt)?,
                created_at: self.created_at,
                updated_at: self.updated_at,
            },
            sealed_headers,
        })
    }
}

struct RequestRow {
    id: Uuid,
    webhook_id: Uuid,
    kind: String,
    delivery_id: Option<String>,
    phase: String,
    message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl RequestRow {
    fn into_request(self) -> Result<WebhookRequest, CoreError> {
        Ok(WebhookRequest {
            id: self.id,
            webhook_id: WebhookId(self.webhook_id),
            kind: RequestKind::parse(&self.kind).ok_or_else(|| corrupt("request kind"))?,
            delivery_id: self.delivery_id,
            phase: RequestPhase::parse(&self.phase).ok_or_else(|| corrupt("request phase"))?,
            message: self.message,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

struct StateColumns<'a> {
    state: &'static str,
    ferriskey_id: Option<&'a str>,
    failure_reason: Option<&'a str>,
}

fn state_columns(state: &WebhookState) -> StateColumns<'_> {
    match state {
        WebhookState::Pending => StateColumns {
            state: "pending",
            ferriskey_id: None,
            failure_reason: None,
        },
        WebhookState::Active { ferriskey_id } => StateColumns {
            state: "active",
            ferriskey_id: Some(ferriskey_id),
            failure_reason: None,
        },
        WebhookState::Failed { reason } => StateColumns {
            state: "failed",
            ferriskey_id: None,
            failure_reason: Some(reason),
        },
    }
}

struct SealedColumns<'a> {
    ciphertext: Option<&'a [u8]>,
    nonce: Option<&'a [u8]>,
    wrapped_dek: Option<&'a str>,
    key_provider: Option<&'a str>,
    key_name: Option<&'a str>,
    key_version: Option<i32>,
}

fn sealed_columns(sealed: Option<&SealedSecret>) -> Result<SealedColumns<'_>, CoreError> {
    let Some(sealed) = sealed else {
        return Ok(SealedColumns {
            ciphertext: None,
            nonce: None,
            wrapped_dek: None,
            key_provider: None,
            key_name: None,
            key_version: None,
        });
    };

    Ok(SealedColumns {
        ciphertext: Some(&sealed.ciphertext),
        nonce: Some(&sealed.nonce),
        wrapped_dek: Some(sealed.wrapped_dek.as_str()),
        key_provider: Some(sealed.key.provider.as_str()),
        key_name: Some(sealed.key.name.as_str()),
        key_version: Some(i32::try_from(sealed.key.version.value()).map_err(|_| {
            CoreError::InternalError("a key version does not fit the column".to_string())
        })?),
    })
}

fn names<'a, T>(items: &'a [T], name: impl Fn(&'a T) -> &'a str) -> Vec<String> {
    items.iter().map(|item| name(item).to_string()).collect()
}

fn revision_column(revision: u64) -> Result<i64, CoreError> {
    i64::try_from(revision)
        .map_err(|_| CoreError::InternalError("a revision does not fit the column".to_string()))
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Webhook, backend = Postgres)]
pub struct PostgresWebhookRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresWebhookRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl WebhookRepository for PostgresWebhookRepository<'_> {
    async fn next_revision(&self, deployment_id: DeploymentId) -> Result<u64, CoreError> {
        let revision = {
            let mut tx = self.tx.lock().await;
            sqlx::query_scalar!(
                r#"
            INSERT INTO deployment_webhook_sync (deployment_id, revision)
            VALUES ($1, 1)
            ON CONFLICT (deployment_id) DO UPDATE
            SET revision = deployment_webhook_sync.revision + 1,
                updated_at = now()
            RETURNING revision
            "#,
                deployment_id.0
            )
            .fetch_one(&mut ***tx)
            .await
        }
        .map_err(db("take the next webhook revision"))?;

        u64::try_from(revision).map_err(corrupt)
    }

    async fn sync_state(&self, deployment_id: DeploymentId) -> Result<WebhookSync, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            SELECT revision, applied_revision, realms
            FROM deployment_webhook_sync
            WHERE deployment_id = $1
            "#,
                deployment_id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read the webhook revisions"))?;

        let Some(row) = row else {
            return Ok(WebhookSync::default());
        };

        Ok(WebhookSync {
            revision: u64::try_from(row.revision).map_err(corrupt)?,
            applied_revision: u64::try_from(row.applied_revision).map_err(corrupt)?,
            realms: row.realms,
        })
    }

    async fn record_applied(
        &self,
        deployment_id: DeploymentId,
        revision: u64,
        realms: &[String],
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let revision = revision_column(revision)?;
        let realms = (!realms.is_empty()).then_some(realms);

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO deployment_webhook_sync (deployment_id, revision, applied_revision, realms, updated_at)
            VALUES ($1, 0, $2, $3, $4)
            ON CONFLICT (deployment_id) DO UPDATE
            SET applied_revision = GREATEST(deployment_webhook_sync.applied_revision, $2),
                realms = $3,
                updated_at = $4
            "#,
                deployment_id.0,
                revision,
                realms as Option<&[String]>,
                at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record an applied webhook revision"))?;

        Ok(())
    }

    async fn list(&self, deployment_id: DeploymentId) -> Result<Vec<StoredWebhook>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                WebhookRow,
                r#"
            SELECT w.id, w.deployment_id, w.realm, w.name, w.description, w.endpoint,
                   w.events, w.header_names,
                   w.headers_ciphertext, w.headers_nonce, w.headers_wrapped_dek,
                   w.headers_key_provider, w.headers_key_name, w.headers_key_version,
                   w.state, w.ferriskey_id, w.failure_reason, w.desired_revision,
                   w.created_at, w.updated_at,
                   s.version AS "secret_version?",
                   s.received_at AS "secret_received_at?"
            FROM deployment_webhooks w
            LEFT JOIN deployment_webhook_secrets s ON s.webhook_id = w.id
            WHERE w.deployment_id = $1
            ORDER BY w.created_at, w.id
            "#,
                deployment_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list webhooks"))?;

        rows.into_iter().map(WebhookRow::into_stored).collect()
    }

    async fn get(
        &self,
        deployment_id: DeploymentId,
        id: WebhookId,
    ) -> Result<Option<StoredWebhook>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                WebhookRow,
                r#"
            SELECT w.id, w.deployment_id, w.realm, w.name, w.description, w.endpoint,
                   w.events, w.header_names,
                   w.headers_ciphertext, w.headers_nonce, w.headers_wrapped_dek,
                   w.headers_key_provider, w.headers_key_name, w.headers_key_version,
                   w.state, w.ferriskey_id, w.failure_reason, w.desired_revision,
                   w.created_at, w.updated_at,
                   s.version AS "secret_version?",
                   s.received_at AS "secret_received_at?"
            FROM deployment_webhooks w
            LEFT JOIN deployment_webhook_secrets s ON s.webhook_id = w.id
            WHERE w.deployment_id = $1 AND w.id = $2
            "#,
                deployment_id.0,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read a webhook"))?;

        row.map(WebhookRow::into_stored).transpose()
    }

    async fn insert(&self, stored: &StoredWebhook) -> Result<(), CoreError> {
        let webhook = &stored.webhook;
        let state = state_columns(&webhook.state);
        let sealed = sealed_columns(stored.sealed_headers.as_ref())?;
        let events = names(&webhook.events, |event| event.name());
        let header_names = names(&webhook.headers, HeaderName::as_str);
        let revision = revision_column(webhook.desired_revision)?;

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO deployment_webhooks (
                id, deployment_id, realm, name, description, endpoint, events, header_names,
                headers_ciphertext, headers_nonce, headers_wrapped_dek,
                headers_key_provider, headers_key_name, headers_key_version,
                state, ferriskey_id, failure_reason, desired_revision, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            "#,
                webhook.id.0,
                webhook.deployment_id.0,
                webhook.realm.as_str(),
                webhook.name.as_str(),
                webhook.description,
                webhook.endpoint.as_str(),
                &events,
                &header_names,
                sealed.ciphertext,
                sealed.nonce,
                sealed.wrapped_dek,
                sealed.key_provider,
                sealed.key_name,
                sealed.key_version,
                state.state,
                state.ferriskey_id,
                state.failure_reason,
                revision,
                webhook.created_at,
                webhook.updated_at,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store a webhook"))?;

        Ok(())
    }

    async fn update(&self, stored: &StoredWebhook) -> Result<bool, CoreError> {
        let webhook = &stored.webhook;
        let state = state_columns(&webhook.state);
        let sealed = sealed_columns(stored.sealed_headers.as_ref())?;
        let events = names(&webhook.events, |event| event.name());
        let header_names = names(&webhook.headers, HeaderName::as_str);
        let revision = revision_column(webhook.desired_revision)?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_webhooks
            SET realm = $3, name = $4, description = $5, endpoint = $6,
                events = $7, header_names = $8,
                headers_ciphertext = $9, headers_nonce = $10, headers_wrapped_dek = $11,
                headers_key_provider = $12, headers_key_name = $13, headers_key_version = $14,
                state = $15, ferriskey_id = $16, failure_reason = $17,
                desired_revision = $18, updated_at = $19
            WHERE id = $1 AND deployment_id = $2
            "#,
                webhook.id.0,
                webhook.deployment_id.0,
                webhook.realm.as_str(),
                webhook.name.as_str(),
                webhook.description,
                webhook.endpoint.as_str(),
                &events,
                &header_names,
                sealed.ciphertext,
                sealed.nonce,
                sealed.wrapped_dek,
                sealed.key_provider,
                sealed.key_name,
                sealed.key_version,
                state.state,
                state.ferriskey_id,
                state.failure_reason,
                revision,
                webhook.updated_at,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("change a webhook"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete(&self, deployment_id: DeploymentId, id: WebhookId) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "DELETE FROM deployment_webhooks WHERE id = $1 AND deployment_id = $2",
                id.0,
                deployment_id.0
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("delete a webhook"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_state(
        &self,
        id: WebhookId,
        state: &WebhookState,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let columns = state_columns(state);

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_webhooks
            SET state = $2, ferriskey_id = $3, failure_reason = $4, updated_at = $5
            WHERE id = $1
            "#,
                id.0,
                columns.state,
                columns.ferriskey_id,
                columns.failure_reason,
                at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record a webhook's state"))?;

        Ok(())
    }

    async fn secret(&self, id: WebhookId) -> Result<Option<SecretRecord>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            SELECT version, ciphertext, nonce, wrapped_dek,
                   key_provider, key_name, key_version, received_at
            FROM deployment_webhook_secrets
            WHERE webhook_id = $1
            "#,
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read a webhook secret"))?;

        row.map(|row| {
            Ok(SecretRecord {
                version: u64::try_from(row.version).map_err(corrupt)?,
                sealed: SealedSecret {
                    ciphertext: row.ciphertext,
                    nonce: row.nonce,
                    wrapped_dek: WrappedDek::new(row.wrapped_dek),
                    key: KeyRef::new(
                        ProviderName::new(row.key_provider),
                        KeyName::new(row.key_name)?,
                        KeyVersion::new(u32::try_from(row.key_version).map_err(corrupt)?),
                    ),
                },
                received_at: row.received_at,
            })
        })
        .transpose()
    }

    async fn store_secret(&self, id: WebhookId, secret: &SecretRecord) -> Result<bool, CoreError> {
        let version = revision_column(secret.version)?;
        let key_version = i32::try_from(secret.sealed.key.version.value()).map_err(|_| {
            CoreError::InternalError("a key version does not fit the column".to_string())
        })?;

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO deployment_webhook_secrets (
                webhook_id, version, ciphertext, nonce, wrapped_dek,
                key_provider, key_name, key_version, received_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (webhook_id) DO UPDATE
            SET version = EXCLUDED.version,
                ciphertext = EXCLUDED.ciphertext,
                nonce = EXCLUDED.nonce,
                wrapped_dek = EXCLUDED.wrapped_dek,
                key_provider = EXCLUDED.key_provider,
                key_name = EXCLUDED.key_name,
                key_version = EXCLUDED.key_version,
                received_at = EXCLUDED.received_at
            WHERE EXCLUDED.version > deployment_webhook_secrets.version
            "#,
                id.0,
                version,
                secret.sealed.ciphertext,
                secret.sealed.nonce,
                secret.sealed.wrapped_dek.as_str(),
                secret.sealed.key.provider.as_str(),
                secret.sealed.key.name.as_str(),
                key_version,
                secret.received_at,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store a webhook secret"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn insert_request(&self, request: &WebhookRequest) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO deployment_webhook_requests
                (id, webhook_id, kind, delivery_id, phase, message, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
                request.id,
                request.webhook_id.0,
                request.kind.as_str(),
                request.delivery_id,
                request.phase.as_str(),
                request.message,
                request.created_at,
                request.updated_at,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record a webhook request"))?;

        Ok(())
    }

    async fn finish_request(
        &self,
        id: Uuid,
        phase: RequestPhase,
        message: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_webhook_requests
            SET phase = $2, message = $3, updated_at = $4
            WHERE id = $1 AND phase = 'pending' AND $2 <> 'pending'
            "#,
                id,
                phase.as_str(),
                message,
                at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("finish a webhook request"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn recent_requests(
        &self,
        deployment_id: DeploymentId,
        per_webhook: usize,
    ) -> Result<Vec<WebhookRequest>, CoreError> {
        let limit = i64::try_from(per_webhook).unwrap_or(i64::MAX);

        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                RequestRow,
                r#"
            SELECT id AS "id!", webhook_id AS "webhook_id!", kind AS "kind!",
                   delivery_id, phase AS "phase!", message,
                   created_at AS "created_at!", updated_at AS "updated_at!"
            FROM (
                SELECT r.*,
                       row_number() OVER (
                           PARTITION BY r.webhook_id ORDER BY r.created_at DESC, r.id DESC
                       ) AS position
                FROM deployment_webhook_requests r
                JOIN deployment_webhooks w ON w.id = r.webhook_id
                WHERE w.deployment_id = $1
            ) ranked
            WHERE position <= $2
            ORDER BY created_at DESC, id DESC
            "#,
                deployment_id.0,
                limit
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list webhook requests"))?;

        rows.into_iter().map(RequestRow::into_request).collect()
    }
}
