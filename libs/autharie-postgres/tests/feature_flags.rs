use autharie_domain::{
    CoreError,
    features::{
        Feature, OverrideTally,
        ports::{FeatureRepository, FlagChange},
    },
    organisation::OrganisationId,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::features::PostgresFeatureRepository;
use chrono::Utc;
use uuid::Uuid;

mod support;
use support::pool;

macro_rules! pool_or_skip {
    () => {
        match pool().await {
            Some(pool) => pool,
            None => {
                eprintln!("skipped: DATABASE_URL is not set");
                return;
            }
        }
    };
}

fn map_err(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

fn change() -> FlagChange {
    FlagChange {
        by: "operator-1".to_string(),
        at: Utc::now(),
    }
}

async fn no_defaults(tx: &SharedTx<'_>) -> Result<(), CoreError> {
    let mut guard = tx.lock().await;
    sqlx::query("DELETE FROM feature_flags")
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;

    Ok(())
}

async fn an_organisation(tx: &SharedTx<'_>) -> Result<OrganisationId, CoreError> {
    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = Uuid::new_v4();

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'flags', $3)")
        .bind(user_id)
        .bind(format!("{user_id}@flags.test"))
        .bind(user_id.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'flags', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
    )
    .bind(organisation_id.0)
    .bind(organisation_id.0.to_string())
    .bind(user_id)
    .execute(&mut ***guard)
    .await
    .map_err(map_err)?;

    Ok(organisation_id)
}

#[tokio::test]
async fn nothing_set_means_off() {
    let pool = pool_or_skip!();

    let enabled: Result<bool, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        no_defaults(&tx).await?;
        let organisation = an_organisation(&tx).await?;
        PostgresFeatureRepository::new(&tx)
            .is_enabled(organisation, Feature::CustomerCloud)
            .await
    })
    .await;

    assert!(!enabled.expect("the transaction"));
}

#[tokio::test]
async fn the_default_applies_to_an_organisation_without_an_override() {
    let pool = pool_or_skip!();

    let enabled: Result<bool, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let features = PostgresFeatureRepository::new(&tx);
        features
            .set_default(Feature::CustomerCloud, true, &change())
            .await?;

        features
            .is_enabled(organisation, Feature::CustomerCloud)
            .await
    })
    .await;

    assert!(enabled.expect("the transaction"));
}

#[tokio::test]
async fn setting_the_default_twice_keeps_the_last_answer() {
    let pool = pool_or_skip!();

    let default: Result<Option<bool>, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        let features = PostgresFeatureRepository::new(&tx);
        features
            .set_default(Feature::CustomerCloud, true, &change())
            .await?;
        features
            .set_default(Feature::CustomerCloud, false, &change())
            .await?;

        Ok(features
            .defaults()
            .await?
            .get(&Feature::CustomerCloud)
            .copied())
    })
    .await;

    assert_eq!(default.expect("the transaction"), Some(false));
}

#[tokio::test]
async fn an_override_beats_the_default_and_clearing_it_gives_the_default_back() {
    let pool = pool_or_skip!();

    let answers: Result<(bool, bool, bool), CoreError> =
        in_scratch_tx(&pool, map_err, async |tx| {
            let organisation = an_organisation(&tx).await?;
            let features = PostgresFeatureRepository::new(&tx);
            features
                .set_default(Feature::CustomerCloud, true, &change())
                .await?;

            features
                .set_override(organisation, Feature::CustomerCloud, Some(false), &change())
                .await?;
            let overridden = features
                .is_enabled(organisation, Feature::CustomerCloud)
                .await?;

            features
                .set_override(organisation, Feature::CustomerCloud, Some(true), &change())
                .await?;
            let replaced = features
                .is_enabled(organisation, Feature::CustomerCloud)
                .await?;

            features
                .set_override(organisation, Feature::CustomerCloud, None, &change())
                .await?;
            features
                .set_default(Feature::CustomerCloud, false, &change())
                .await?;
            let cleared = features
                .is_enabled(organisation, Feature::CustomerCloud)
                .await?;

            Ok((overridden, replaced, cleared))
        })
        .await;

    assert_eq!(answers.expect("the transaction"), (false, true, false));
}

#[tokio::test]
async fn an_override_belongs_to_one_organisation() {
    let pool = pool_or_skip!();

    let answers: Result<(bool, bool), CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        no_defaults(&tx).await?;
        let with = an_organisation(&tx).await?;
        let without = an_organisation(&tx).await?;
        let features = PostgresFeatureRepository::new(&tx);
        features
            .set_override(with, Feature::CustomerCloud, Some(true), &change())
            .await?;

        Ok((
            features.is_enabled(with, Feature::CustomerCloud).await?,
            features.is_enabled(without, Feature::CustomerCloud).await?,
        ))
    })
    .await;

    assert_eq!(answers.expect("the transaction"), (true, false));
}

#[tokio::test]
async fn the_enabled_list_holds_only_what_resolves_to_on() {
    let pool = pool_or_skip!();

    let enabled: Result<(Vec<Feature>, Vec<Feature>), CoreError> =
        in_scratch_tx(&pool, map_err, async |tx| {
            no_defaults(&tx).await?;
            let on = an_organisation(&tx).await?;
            let off = an_organisation(&tx).await?;
            let features = PostgresFeatureRepository::new(&tx);
            features
                .set_override(on, Feature::CustomerCloud, Some(true), &change())
                .await?;

            Ok((
                features.enabled_for(on).await?,
                features.enabled_for(off).await?,
            ))
        })
        .await;

    let (on, off) = enabled.expect("the transaction");
    assert_eq!(on, vec![Feature::CustomerCloud]);
    assert!(off.is_empty());
}

#[tokio::test]
async fn overrides_are_counted_per_direction() {
    let pool = pool_or_skip!();

    let before_after: Result<(OverrideTally, OverrideTally), CoreError> =
        in_scratch_tx(&pool, map_err, async |tx| {
            let features = PostgresFeatureRepository::new(&tx);
            let before = features
                .override_tallies()
                .await?
                .get(&Feature::CustomerCloud)
                .copied()
                .unwrap_or_default();

            let first = an_organisation(&tx).await?;
            let second = an_organisation(&tx).await?;
            let third = an_organisation(&tx).await?;
            features
                .set_override(first, Feature::CustomerCloud, Some(true), &change())
                .await?;
            features
                .set_override(second, Feature::CustomerCloud, Some(true), &change())
                .await?;
            features
                .set_override(third, Feature::CustomerCloud, Some(false), &change())
                .await?;

            let after = features
                .override_tallies()
                .await?
                .get(&Feature::CustomerCloud)
                .copied()
                .unwrap_or_default();

            Ok((before, after))
        })
        .await;

    let (before, after) = before_after.expect("the transaction");
    assert_eq!(after.on, before.on + 2);
    assert_eq!(after.off, before.off + 1);
}

#[tokio::test]
async fn a_key_the_code_no_longer_defines_is_ignored_rather_than_fatal() {
    let pool = pool_or_skip!();

    let read: Result<(usize, bool), CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        no_defaults(&tx).await?;
        let organisation = an_organisation(&tx).await?;
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO feature_flags (key, enabled_by_default, updated_at, updated_by) \
                 VALUES ('removed_in_a_later_release', true, now(), 'x') \
                 ON CONFLICT (key) DO NOTHING",
            )
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
            sqlx::query(
                "INSERT INTO organisation_feature_flags \
                 (organisation_id, key, enabled, updated_at, updated_by) \
                 VALUES ($1, 'removed_in_a_later_release', true, now(), 'x')",
            )
            .bind(organisation.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        let features = PostgresFeatureRepository::new(&tx);
        features.defaults().await?;
        features.override_tallies().await?;

        Ok((
            features.overrides(organisation).await?.len(),
            features
                .is_enabled(organisation, Feature::CustomerCloud)
                .await?,
        ))
    })
    .await;

    let (overrides, customer_cloud) = read.expect("the transaction");
    assert_eq!(overrides, 0);
    assert!(!customer_cloud);
}

#[tokio::test]
async fn deleting_an_organisation_takes_its_overrides_with_it() {
    let pool = pool_or_skip!();

    let remaining: Result<i64, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        PostgresFeatureRepository::new(&tx)
            .set_override(organisation, Feature::CustomerCloud, Some(true), &change())
            .await?;

        let mut guard = tx.lock().await;
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(organisation.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;

        sqlx::query_scalar(
            "SELECT COUNT(*) FROM organisation_feature_flags WHERE organisation_id = $1",
        )
        .bind(organisation.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(map_err)
    })
    .await;

    assert_eq!(remaining.expect("the transaction"), 0);
}
