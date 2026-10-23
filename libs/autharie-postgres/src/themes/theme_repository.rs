use autharie_domain::{
    CoreError,
    deployments::DeploymentId,
    themes::{
        Theme, ThemeError, ThemeId, ThemeName, ThemeRealm, ThemeState, ThemeSync, ThemeTokens,
        ports::ThemeRepository,
    },
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

const NAME_CONSTRAINT: &str = "deployment_portal_themes_name_unique";

fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {what}: {error}"),
    }
}

fn corrupt(what: impl std::fmt::Display) -> CoreError {
    CoreError::InternalError(format!("a stored theme cannot be read: {what}"))
}

fn name_taken(theme: &Theme, what: &'static str) -> impl FnOnce(sqlx::Error) -> CoreError {
    let name = theme.name.as_str().to_string();

    move |error| match &error {
        sqlx::Error::Database(failure) if failure.constraint() == Some(NAME_CONSTRAINT) => {
            ThemeError::NameTaken { name }.into()
        }
        _ => db(what)(error),
    }
}

struct ThemeRow {
    id: Uuid,
    deployment_id: Uuid,
    realm: String,
    name: String,
    tokens: serde_json::Value,
    active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ThemeRow {
    fn into_theme(self) -> Result<Theme, CoreError> {
        Ok(Theme {
            id: ThemeId(self.id),
            deployment_id: DeploymentId(self.deployment_id),
            realm: ThemeRealm::parse(&self.realm).map_err(corrupt)?,
            name: ThemeName::parse(&self.name).map_err(corrupt)?,
            tokens: ThemeTokens::parse(&self.tokens).map_err(corrupt)?,
            active: self.active,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

struct StateColumns<'a> {
    phase: &'static str,
    message: Option<&'a str>,
}

fn state_columns(state: &ThemeState) -> StateColumns<'_> {
    match state {
        ThemeState::Pending => StateColumns {
            phase: "pending",
            message: None,
        },
        ThemeState::Applied => StateColumns {
            phase: "applied",
            message: None,
        },
        ThemeState::Failed { message } => StateColumns {
            phase: "failed",
            message: Some(message),
        },
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Theme, backend = Postgres)]
pub struct PostgresThemeRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresThemeRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl ThemeRepository for PostgresThemeRepository<'_> {
    async fn lock_realm(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!(
                    "portal-themes:{}:{}",
                    deployment_id.0,
                    realm.as_str()
                ))
                .execute(&mut ***tx)
                .await
        }
        .map_err(db("lock a realm's themes"))?;

        Ok(())
    }

    async fn list(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<Vec<Theme>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                ThemeRow,
                r#"
            SELECT id, deployment_id, realm, name, tokens, active, created_at, updated_at
            FROM deployment_portal_themes
            WHERE deployment_id = $1 AND realm = $2
            ORDER BY created_at, id
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(db("list a realm's themes"))?;

        rows.into_iter().map(ThemeRow::into_theme).collect()
    }

    async fn get(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
    ) -> Result<Option<Theme>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                ThemeRow,
                r#"
            SELECT id, deployment_id, realm, name, tokens, active, created_at, updated_at
            FROM deployment_portal_themes
            WHERE deployment_id = $1 AND realm = $2 AND id = $3
            "#,
                deployment_id.0,
                realm.as_str(),
                id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read a theme"))?;

        row.map(ThemeRow::into_theme).transpose()
    }

    async fn active(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<Option<Theme>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                ThemeRow,
                r#"
            SELECT id, deployment_id, realm, name, tokens, active, created_at, updated_at
            FROM deployment_portal_themes
            WHERE deployment_id = $1 AND realm = $2 AND active
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read the active theme"))?;

        row.map(ThemeRow::into_theme).transpose()
    }

    async fn insert(&self, theme: &Theme) -> Result<(), CoreError> {
        let tokens = serde_json::to_value(&theme.tokens)
            .map_err(|error| CoreError::InternalError(error.to_string()))?;

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            INSERT INTO deployment_portal_themes
                (id, deployment_id, realm, name, tokens, active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
                theme.id.0,
                theme.deployment_id.0,
                theme.realm.as_str(),
                theme.name.as_str(),
                tokens,
                theme.active,
                theme.created_at,
                theme.updated_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(name_taken(theme, "store a theme"))?;

        Ok(())
    }

    async fn update(&self, theme: &Theme) -> Result<(), CoreError> {
        let tokens = serde_json::to_value(&theme.tokens)
            .map_err(|error| CoreError::InternalError(error.to_string()))?;

        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_portal_themes
            SET name = $4, tokens = $5, updated_at = $6
            WHERE deployment_id = $1 AND realm = $2 AND id = $3
            "#,
                theme.deployment_id.0,
                theme.realm.as_str(),
                theme.id.0,
                theme.name.as_str(),
                tokens,
                theme.updated_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(name_taken(theme, "change a theme"))?;

        Ok(())
    }

    async fn delete(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
    ) -> Result<bool, CoreError> {
        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            DELETE FROM deployment_portal_themes
            WHERE deployment_id = $1 AND realm = $2 AND id = $3
            "#,
                deployment_id.0,
                realm.as_str(),
                id.0
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("delete a theme"))?;

        Ok(result.rows_affected() > 0)
    }

    async fn activate(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
        at: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut tx = self.tx.lock().await;

        sqlx::query!(
            r#"
            UPDATE deployment_portal_themes
            SET active = false, updated_at = $4
            WHERE deployment_id = $1 AND realm = $2 AND active AND id <> $3
            "#,
            deployment_id.0,
            realm.as_str(),
            id.0,
            at
        )
        .execute(&mut ***tx)
        .await
        .map_err(db("deactivate the previous theme"))?;

        let activated = sqlx::query!(
            r#"
            UPDATE deployment_portal_themes
            SET active = true, updated_at = $4
            WHERE deployment_id = $1 AND realm = $2 AND id = $3
            "#,
            deployment_id.0,
            realm.as_str(),
            id.0,
            at
        )
        .execute(&mut ***tx)
        .await
        .map_err(db("activate a theme"))?;

        Ok(activated.rows_affected() > 0)
    }

    async fn deactivate(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_portal_themes
            SET active = false, updated_at = $3
            WHERE deployment_id = $1 AND realm = $2 AND active
            "#,
                deployment_id.0,
                realm.as_str(),
                at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("deactivate a realm's theme"))?;

        Ok(())
    }

    async fn next_revision(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<u64, CoreError> {
        let revision = {
            let mut tx = self.tx.lock().await;
            sqlx::query_scalar!(
                r#"
            INSERT INTO deployment_portal_theme_sync (deployment_id, realm, revision, phase, message)
            VALUES ($1, $2, 1, 'pending', NULL)
            ON CONFLICT (deployment_id, realm) DO UPDATE
            SET revision = deployment_portal_theme_sync.revision + 1,
                phase = 'pending',
                message = NULL,
                updated_at = now()
            RETURNING revision
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_one(&mut ***tx)
            .await
        }
        .map_err(db("take the next theme revision"))?;

        u64::try_from(revision).map_err(corrupt)
    }

    async fn sync(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<Option<ThemeSync>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            SELECT revision, phase, message
            FROM deployment_portal_theme_sync
            WHERE deployment_id = $1 AND realm = $2
            "#,
                deployment_id.0,
                realm.as_str()
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(db("read the theme revision"))?;

        row.map(|row| {
            let state = match row.phase.as_str() {
                "pending" => ThemeState::Pending,
                "applied" => ThemeState::Applied,
                "failed" => ThemeState::Failed {
                    message: row.message.unwrap_or_default(),
                },
                other => return Err(corrupt(format!("phase '{other}'"))),
            };

            Ok(ThemeSync {
                state,
                revision: u64::try_from(row.revision).map_err(corrupt)?,
            })
        })
        .transpose()
    }

    async fn set_state(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        state: &ThemeState,
    ) -> Result<bool, CoreError> {
        let columns = state_columns(state);

        let result = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
            UPDATE deployment_portal_theme_sync
            SET phase = $3, message = $4, updated_at = now()
            WHERE deployment_id = $1 AND realm = $2
            "#,
                deployment_id.0,
                realm.as_str(),
                columns.phase,
                columns.message
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(db("record the state of a realm's theme"))?;

        Ok(result.rows_affected() > 0)
    }
}
