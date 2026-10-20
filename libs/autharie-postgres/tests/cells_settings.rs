use autharie_domain::{
    CoreError,
    cells::{CellsSettings, CellsSettingsRepository},
    organisation::OrganisationId,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::cells::PostgresCellsSettingsRepository;
use chrono::Utc;
use uuid::Uuid;

mod support;
use support::pool;

fn db_error(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, db_error, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

async fn seed_organisation(tx: &SharedTx<'_>) -> Result<OrganisationId, CoreError> {
    let user = Uuid::new_v4();
    let organisation = Uuid::new_v4();
    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'cells', $3)")
        .bind(user)
        .bind(format!("{user}@cells-settings.test"))
        .bind(user.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(db_error)?;
    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'cells', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
    )
    .bind(organisation)
    .bind(format!("cs-{organisation}"))
    .bind(user)
    .execute(&mut ***guard)
    .await
    .map_err(db_error)?;

    Ok(OrganisationId(organisation))
}

async fn clear(tx: &SharedTx<'_>) -> Result<(), CoreError> {
    PostgresCellsSettingsRepository::new(tx)
        .set(None, "reset", Utc::now())
        .await
        .map(|_| ())
}

#[tokio::test]
async fn the_settings_start_without_an_organisation() {
    scratch(async |tx| {
        clear(&tx).await?;
        let settings = PostgresCellsSettingsRepository::new(&tx).get().await?;

        assert_eq!(settings.organisation_id, None);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn an_organisation_is_stored_with_who_set_it_and_when() {
    scratch(async |tx| {
        let organisation = seed_organisation(&tx).await?;
        let repository = PostgresCellsSettingsRepository::new(&tx);
        let at = Utc::now();

        let written = repository.set(Some(organisation), "operator-1", at).await?;
        let read: CellsSettings = repository.get().await?;

        assert_eq!(written, read);
        assert_eq!(read.organisation_id, Some(organisation));
        assert_eq!(read.updated_by.as_deref(), Some("operator-1"));
        assert_eq!(read.updated_at.timestamp_micros(), at.timestamp_micros());
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn the_organisation_can_be_cleared_again() {
    scratch(async |tx| {
        let organisation = seed_organisation(&tx).await?;
        let repository = PostgresCellsSettingsRepository::new(&tx);

        repository
            .set(Some(organisation), "operator-1", Utc::now())
            .await?;
        let cleared = repository.set(None, "operator-2", Utc::now()).await?;

        assert_eq!(cleared.organisation_id, None);
        assert_eq!(cleared.updated_by.as_deref(), Some("operator-2"));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn an_organisation_that_does_not_exist_is_refused_by_the_database() {
    scratch(async |tx| {
        let refused = PostgresCellsSettingsRepository::new(&tx)
            .set(Some(OrganisationId(Uuid::new_v4())), "operator", Utc::now())
            .await;

        assert!(matches!(refused, Err(CoreError::DatabaseError { .. })));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn there_is_only_ever_one_row() {
    scratch(async |tx| {
        let mut guard = tx.lock().await;
        let second = sqlx::query("INSERT INTO platform_cells_settings (id) VALUES (FALSE)")
            .execute(&mut ***guard)
            .await;
        assert!(second.is_err());
        Ok(())
    })
    .await;
}
