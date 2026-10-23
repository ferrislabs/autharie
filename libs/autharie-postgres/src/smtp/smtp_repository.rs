use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    dataplane::credential_repository::SealedSecret,
    deployments::DeploymentId,
    smtp::{
        FromEmail, FromName, SmtpConfig, SmtpEncryption, SmtpHost, SmtpPort, SmtpRealm,
        SmtpSettings, SmtpState, SmtpUsername, StoredSmtp, ports::SmtpRepository,
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
    CoreError::InternalError(format!("a stored email provider cannot be read: {what}"))
}

struct SmtpRow {
    deployment_id: Uuid,
    realm: String,
    host: String,
    port: i32,
    encryption: String,
    username: String,
    from_email: String,
    from_name: String,
    password_ciphertext: Vec<u8>,
    password_nonce: Vec<u8>,
    password_wrapped_dek: String,
    password_key_provider: String,
    password_key_name: String,
    password_key_version: i32,
    phase: String,
    message: Option<String>,
    updated_at: DateTime<Utc>,
}

impl SmtpRow {
    fn into_stored(self) -> Result<StoredSmtp, CoreError> {
        let state = match self.phase.as_str() {
            "pending" => SmtpState::Pending,
            "applied" => SmtpState::Applied,
            "failed" => SmtpState::Failed {
                message: self.message.unwrap_or_default(),
            },
            other => return Err(corrupt(format!("phase '{other}'"))),
        };

        Ok(StoredSmtp {
            config: SmtpConfig {
                deployment_id: DeploymentId(self.deployment_id),
                realm: SmtpRealm::parse(&self.realm).map_err(corrupt)?,
                settings: SmtpSettings {
                    host: SmtpHost::parse(&self.host).map_err(corrupt)?,
                    port: SmtpPort::parse(i64::from(self.port)).map_err(corrupt)?,
                    encryption: SmtpEncryption::parse(&self.encryption).map_err(corrupt)?,
                    username: SmtpUsername::parse(&self.username).map_err(corrupt)?,
                    from_email: FromEmail::parse(&self.from_email).map_err(corrupt)?,
                    from_name: FromName::parse(&self.from_name).map_err(corrupt)?,
                },
                state,
                updated_at: self.updated_at,
            },
            sealed_password: SealedSecret {
                ciphertext: self.password_ciphertext,
                nonce: self.password_nonce,
                wrapped_dek: WrappedDek::new(self.password_wrapped_dek),
                key: KeyRef::new(
                    ProviderName::new(self.password_key_provider),
                    KeyName::new(self.password_key_name)?,
                    KeyVersion::new(u32::try_from(self.password_key_version).map_err(corrupt)?),
                ),
            },
        })
    }
}

struct StateColumns<'a> {
    phase: &'static str,
    message: Option<&'a str>,
}

fn state_columns(state: &SmtpState) -> StateColumns<'_> {
    match state {
        SmtpState::Pending => StateColumns {
            phase: "pending",
            message: None,
        },
        SmtpState::Applied => StateColumns {
            phase: "applied",
            message: None,
        },
        SmtpState::Failed { message } => StateColumns {
            phase: "failed",
            message: Some(message),
        },
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Smtp, backend = Postgres)]
pub struct PostgresSmtpRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresSmtpRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl SmtpRepository for PostgresSmtpRepository<'_> {
    async fn next_revision(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> Result<u64, CoreError> {
        let revision = {
            let mut tx = self.tx.lock().await;
            sqlx::query_scalar!(
                r#"
            INSERT INTO deployment_smtp_sync (deployment_id, realm, revision)
            VALUES ($1, $2, 1)
            ON CONFLICT (deployment_id, realm) DO UPDATE
            SET revision = deployment_smtp_sync.revision + 1,
                updated_at = now()
            RETURNING revision
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_one(&mut ***tx)
            .await
        }
        .map_err(db("take the next email provider revision"))?;

        u64::try_from(revision).map_err(corrupt)
    }

    async fn revision(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> Result<u64, CoreError> {
        let revision = {
            let mut tx = self.tx.lock().await;
            sqlx::query_scalar!(
                r#"
            SELECT revision
            FROM deployment_smtp_sync
            WHERE deployment_id = $1 AND realm = $2
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read the email provider revision"))?;

        revision.map_or(Ok(0), |revision| u64::try_from(revision).map_err(corrupt))
    }

    async fn get(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> Result<Option<StoredSmtp>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SmtpRow,
                r#"
            SELECT deployment_id, realm, host, port, encryption, username, from_email, from_name,
                   password_ciphertext, password_nonce, password_wrapped_dek,
                   password_key_provider, password_key_name, password_key_version,
                   phase, message, updated_at
            FROM deployment_smtp_configs
            WHERE deployment_id = $1 AND realm = $2
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read an email provider"))?;

        row.map(SmtpRow::into_stored).transpose()
    }

    async fn upsert(&self, stored: &StoredSmtp) -> Result<(), CoreError> {
        let config = &stored.config;
        let settings = &config.settings;
        let sealed = &stored.sealed_password;
        let state = state_columns(&config.state);
        let key_version = i32::try_from(sealed.key.version.value()).map_err(|_| {
            CoreError::InternalError("a key version does not fit the column".to_string())
        })?;

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO deployment_smtp_configs (
                deployment_id, realm, host, port, encryption, username, from_email, from_name,
                password_ciphertext, password_nonce, password_wrapped_dek,
                password_key_provider, password_key_name, password_key_version,
                phase, message, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            ON CONFLICT (deployment_id, realm) DO UPDATE
            SET host = EXCLUDED.host,
                port = EXCLUDED.port,
                encryption = EXCLUDED.encryption,
                username = EXCLUDED.username,
                from_email = EXCLUDED.from_email,
                from_name = EXCLUDED.from_name,
                password_ciphertext = EXCLUDED.password_ciphertext,
                password_nonce = EXCLUDED.password_nonce,
                password_wrapped_dek = EXCLUDED.password_wrapped_dek,
                password_key_provider = EXCLUDED.password_key_provider,
                password_key_name = EXCLUDED.password_key_name,
                password_key_version = EXCLUDED.password_key_version,
                phase = EXCLUDED.phase,
                message = EXCLUDED.message,
                updated_at = EXCLUDED.updated_at
            "#,
                config.deployment_id.0,
                config.realm.as_str(),
                settings.host.as_str(),
                i32::from(settings.port.value()),
                settings.encryption.as_str(),
                settings.username.as_str(),
                settings.from_email.as_str(),
                settings.from_name.as_str(),
                sealed.ciphertext,
                sealed.nonce,
                sealed.wrapped_dek.as_str(),
                sealed.key.provider.as_str(),
                sealed.key.name.as_str(),
                key_version,
                state.phase,
                state.message,
                config.updated_at,
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("store an email provider"))?;

        Ok(())
    }

    async fn delete(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "DELETE FROM deployment_smtp_configs WHERE deployment_id = $1 AND realm = $2",
                deployment_id.0,
                realm.as_str()
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("delete an email provider"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_state(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
        state: &SmtpState,
        at: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let columns = state_columns(state);

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_smtp_configs
            SET phase = $3, message = $4, updated_at = $5
            WHERE deployment_id = $1 AND realm = $2
            "#,
                deployment_id.0,
                realm.as_str(),
                columns.phase,
                columns.message,
                at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record an email provider's state"))?;

        Ok(result.rows_affected() > 0)
    }
}
