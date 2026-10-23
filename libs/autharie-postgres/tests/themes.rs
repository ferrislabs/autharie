use autharie_domain::{
    CoreError,
    dataplane::{
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        network::NetworkAccess, ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    themes::{
        Theme, ThemeError, ThemeId, ThemeName, ThemeRealm, ThemeState, ThemeSync, ThemeTokens,
        ports::ThemeRepository,
    },
    upgrades::policy::AutoUpgradePolicy,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    dataplane::PostgresDataPlaneRepository, deployments::PostgresDeploymentRepository,
    themes::PostgresThemeRepository,
};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

mod support;
use support::pool;

fn db(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

fn realm(name: &str) -> ThemeRealm {
    ThemeRealm::parse(name).unwrap()
}

fn tokens(primary: &str) -> ThemeTokens {
    ThemeTokens::parse(&json!({
        "colors": { "primaryButton": primary },
        "fonts": { "baseSize": 16, "links": { "weight": 600, "sizePct": 87.5, "style": "underline" } },
        "borders": { "widgetShadow": "small" }
    }))
    .unwrap()
}

fn theme(deployment: DeploymentId, realm_name: &str, name: &str, age_seconds: i64) -> Theme {
    let at = Utc::now() - Duration::seconds(age_seconds);
    Theme {
        id: ThemeId(Uuid::new_v4()),
        deployment_id: deployment,
        realm: realm(realm_name),
        name: ThemeName::parse(name).unwrap(),
        tokens: tokens("#112233"),
        active: false,
        created_at: at,
        updated_at: at,
    }
}

fn is_name_taken(error: &CoreError) -> bool {
    matches!(error, CoreError::Theme(ThemeError::NameTaken { .. }))
}

#[tokio::test]
async fn a_theme_survives_the_round_trip_with_its_tokens() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let stored = theme(deployment.id, "master", "Autumn", 0);

        repository.insert(&stored).await?;

        Ok((
            stored.clone(),
            repository
                .get(deployment.id, &realm("master"), stored.id)
                .await?,
            repository.list(deployment.id, &realm("master")).await?,
        ))
    })
    .await;

    let (stored, got, listed) = read.expect("the transaction");
    let got = got.expect("found");
    assert_eq!(got.name, stored.name);
    assert_eq!(got.tokens, stored.tokens);
    assert!(!got.active);
    assert_eq!(got.realm, realm("master"));
    assert_eq!(listed.len(), 1);
}

#[tokio::test]
async fn a_theme_is_found_only_under_its_own_deployment_and_realm() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let stored = theme(deployment.id, "master", "Autumn", 0);
        repository.insert(&stored).await?;

        Ok((
            repository
                .get(DeploymentId(Uuid::new_v4()), &realm("master"), stored.id)
                .await?,
            repository
                .get(deployment.id, &realm("customers"), stored.id)
                .await?,
            repository.list(deployment.id, &realm("customers")).await?,
            repository
                .delete(deployment.id, &realm("customers"), stored.id)
                .await?,
            repository
                .activate(deployment.id, &realm("customers"), stored.id, Utc::now())
                .await?,
        ))
    })
    .await;

    let (other_deployment, other_realm, listed, deleted, activated) =
        read.expect("the transaction");
    assert!(other_deployment.is_none());
    assert!(other_realm.is_none());
    assert!(listed.is_empty());
    assert!(!deleted);
    assert!(!activated);
}

#[tokio::test]
async fn themes_are_listed_oldest_first() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        for (name, age) in [("Second", 10), ("Third", 5), ("First", 20)] {
            repository
                .insert(&theme(deployment.id, "master", name, age))
                .await?;
        }

        repository.list(deployment.id, &realm("master")).await
    })
    .await;

    let names: Vec<String> = read
        .expect("the transaction")
        .into_iter()
        .map(|theme| theme.name.as_str().to_string())
        .collect();
    assert_eq!(names, vec!["First", "Second", "Third"]);
}

#[tokio::test]
async fn a_name_is_unique_per_realm_whatever_its_case() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        repository
            .insert(&theme(deployment.id, "master", "Autumn", 0))
            .await?;

        let case = {
            let mut guard = tx.lock().await;
            sqlx::query("SAVEPOINT clash")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            drop(guard);
            let outcome = repository
                .insert(&theme(deployment.id, "master", "aUTUMN", 0))
                .await;
            let mut guard = tx.lock().await;
            sqlx::query("ROLLBACK TO SAVEPOINT clash")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            outcome
        };

        let other_realm = repository
            .insert(&theme(deployment.id, "customers", "Autumn", 0))
            .await;

        let winter = theme(deployment.id, "master", "Winter", 0);
        repository.insert(&winter).await?;
        let mut renamed = winter.clone();
        renamed.name = ThemeName::parse("AUTUMN").unwrap();
        let rename = {
            let mut guard = tx.lock().await;
            sqlx::query("SAVEPOINT rename")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            drop(guard);
            let outcome = repository.update(&renamed).await;
            let mut guard = tx.lock().await;
            sqlx::query("ROLLBACK TO SAVEPOINT rename")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            outcome
        };

        Ok((case, other_realm, rename))
    })
    .await;

    let (case, other_realm, rename) = read.expect("the transaction");
    assert!(is_name_taken(&case.expect_err("refused")));
    assert!(other_realm.is_ok());
    assert!(is_name_taken(&rename.expect_err("refused")));
}

#[tokio::test]
async fn an_update_changes_the_name_and_tokens_and_nothing_else() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let original = theme(deployment.id, "master", "Autumn", 30);
        repository.insert(&original).await?;
        repository
            .activate(deployment.id, &realm("master"), original.id, Utc::now())
            .await?;

        let mut changed = original.clone();
        changed.name = ThemeName::parse("Fall").unwrap();
        changed.tokens = tokens("#abcdef");
        changed.updated_at = Utc::now();
        repository.update(&changed).await?;

        Ok((
            original.clone(),
            repository
                .get(deployment.id, &realm("master"), original.id)
                .await?,
        ))
    })
    .await;

    let (original, got) = read.expect("the transaction");
    let got = got.expect("found");
    assert_eq!(got.name.as_str(), "Fall");
    assert_eq!(got.tokens, tokens("#abcdef"));
    assert!(got.active);
    assert_eq!(got.created_at.timestamp(), original.created_at.timestamp());
}

#[tokio::test]
async fn a_realm_has_at_most_one_active_theme_whatever_writes_to_the_table() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let mut first = theme(deployment.id, "master", "Autumn", 0);
        first.active = true;
        repository.insert(&first).await?;

        let mut second = theme(deployment.id, "master", "Winter", 0);
        second.active = true;
        let direct = {
            let mut guard = tx.lock().await;
            sqlx::query("SAVEPOINT two_active")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            drop(guard);
            let outcome = repository.insert(&second).await;
            let mut guard = tx.lock().await;
            sqlx::query("ROLLBACK TO SAVEPOINT two_active")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            outcome
        };

        let mut elsewhere = theme(deployment.id, "customers", "Autumn", 0);
        elsewhere.active = true;
        let other_realm = repository.insert(&elsewhere).await;

        Ok((direct, other_realm))
    })
    .await;

    let (direct, other_realm) = read.expect("the transaction");
    assert!(direct.is_err());
    assert!(other_realm.is_ok());
}

#[tokio::test]
async fn activating_swaps_the_active_theme_and_deactivating_clears_it() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let master = realm("master");
        let customers = realm("customers");
        let first = theme(deployment.id, "master", "Autumn", 20);
        let second = theme(deployment.id, "master", "Winter", 10);
        let elsewhere = theme(deployment.id, "customers", "Autumn", 5);
        for stored in [&first, &second, &elsewhere] {
            repository.insert(stored).await?;
        }

        let none_yet = repository.active(deployment.id, &master).await?;
        repository
            .activate(deployment.id, &master, first.id, Utc::now())
            .await?;
        repository
            .activate(deployment.id, &customers, elsewhere.id, Utc::now())
            .await?;
        let first_active = repository.active(deployment.id, &master).await?;
        let swapped = repository
            .activate(deployment.id, &master, second.id, Utc::now())
            .await?;
        let second_active = repository.active(deployment.id, &master).await?;
        let first_after = repository.get(deployment.id, &master, first.id).await?;
        let again = repository
            .activate(deployment.id, &master, second.id, Utc::now())
            .await?;
        repository
            .deactivate(deployment.id, &master, Utc::now())
            .await?;
        let cleared = repository.active(deployment.id, &master).await?;
        let untouched = repository.active(deployment.id, &customers).await?;
        let missing = repository
            .activate(deployment.id, &master, ThemeId(Uuid::new_v4()), Utc::now())
            .await?;

        Ok((
            (first, second, elsewhere),
            (none_yet, first_active, swapped, second_active, first_after),
            (again, cleared, untouched, missing),
        ))
    })
    .await;

    let (
        (first, second, elsewhere),
        (none_yet, first_active, swapped, second_active, first_after),
        (again, cleared, untouched, missing),
    ) = read.expect("the transaction");
    assert!(none_yet.is_none());
    assert_eq!(first_active.expect("active").id, first.id);
    assert!(swapped);
    assert_eq!(second_active.expect("active").id, second.id);
    assert!(!first_after.expect("kept").active);
    assert!(again);
    assert!(cleared.is_none());
    assert_eq!(untouched.expect("active").id, elsewhere.id);
    assert!(!missing);
}

#[tokio::test]
async fn deleting_reports_whether_a_theme_went() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let stored = theme(deployment.id, "master", "Autumn", 0);
        repository.insert(&stored).await?;

        let first = repository
            .delete(deployment.id, &realm("master"), stored.id)
            .await?;
        let second = repository
            .delete(deployment.id, &realm("master"), stored.id)
            .await?;
        let left = repository.list(deployment.id, &realm("master")).await?;

        Ok((first, second, left))
    })
    .await;

    let (first, second, left) = read.expect("the transaction");
    assert!(first);
    assert!(!second);
    assert!(left.is_empty());
}

#[tokio::test]
async fn the_revision_counts_up_per_realm_and_never_goes_back_after_a_delete() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let master = realm("master");
        let customers = realm("customers");

        let before = repository.sync(deployment.id, &master).await?;
        let stored = theme(deployment.id, "master", "Autumn", 0);
        repository.insert(&stored).await?;
        let first = repository.next_revision(deployment.id, &master).await?;
        let second = repository.next_revision(deployment.id, &master).await?;
        let other_realm = repository.next_revision(deployment.id, &customers).await?;

        repository.delete(deployment.id, &master, stored.id).await?;
        let after_delete = repository.sync(deployment.id, &master).await?;
        let recreated = repository.next_revision(deployment.id, &master).await?;

        Ok((before, first, second, other_realm, after_delete, recreated))
    })
    .await;

    let (before, first, second, other_realm, after_delete, recreated) =
        read.expect("the transaction");
    assert!(before.is_none());
    assert_eq!((first, second), (1, 2));
    assert_eq!(other_realm, 1);
    assert_eq!(after_delete.expect("kept").revision, 2);
    assert_eq!(recreated, 3);
}

#[tokio::test]
async fn a_new_revision_starts_pending_and_a_state_is_recorded_with_its_message() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        let master = realm("master");

        let unknown = repository
            .set_state(deployment.id, &master, &ThemeState::Applied)
            .await?;
        repository.next_revision(deployment.id, &master).await?;
        let pending = repository.sync(deployment.id, &master).await?;
        let failed = ThemeState::Failed {
            message: "404 portal themes".to_string(),
        };
        let set = repository
            .set_state(deployment.id, &master, &failed)
            .await?;
        let failed_row = repository.sync(deployment.id, &master).await?;
        repository
            .set_state(deployment.id, &master, &ThemeState::Applied)
            .await?;
        let applied_row = repository.sync(deployment.id, &master).await?;
        repository.next_revision(deployment.id, &master).await?;
        let renewed = repository.sync(deployment.id, &master).await?;

        Ok((unknown, pending, set, failed_row, applied_row, renewed))
    })
    .await;

    let (unknown, pending, set, failed_row, applied_row, renewed) = read.expect("the transaction");
    assert!(!unknown);
    assert_eq!(
        pending,
        Some(ThemeSync {
            state: ThemeState::Pending,
            revision: 1
        })
    );
    assert!(set);
    assert_eq!(
        failed_row,
        Some(ThemeSync {
            state: ThemeState::Failed {
                message: "404 portal themes".to_string()
            },
            revision: 1
        })
    );
    assert_eq!(applied_row.expect("kept").state, ThemeState::Applied);
    assert_eq!(
        renewed,
        Some(ThemeSync {
            state: ThemeState::Pending,
            revision: 2
        })
    );
}

#[tokio::test]
async fn locking_a_realm_can_be_repeated_within_a_transaction() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);

        repository
            .lock_realm(deployment.id, &realm("master"))
            .await?;
        repository.lock_realm(deployment.id, &realm("master")).await
    })
    .await;

    read.expect("the transaction");
}

#[tokio::test]
async fn the_table_refuses_tokens_that_are_not_an_object_or_are_too_large() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let mut outcomes = Vec::new();

        for (index, bad) in [json!([1]), json!("x"), json!({ "pad": "x".repeat(33_000) })]
            .into_iter()
            .enumerate()
        {
            let mut guard = tx.lock().await;
            sqlx::query("SAVEPOINT bad_tokens")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            let outcome = sqlx::query(
                "INSERT INTO deployment_portal_themes \
                 (id, deployment_id, realm, name, tokens, created_at, updated_at) \
                 VALUES ($1, $2, 'master', $3, $4, now(), now())",
            )
            .bind(Uuid::new_v4())
            .bind(deployment.id.0)
            .bind(format!("bad {index}"))
            .bind(bad)
            .execute(&mut ***guard)
            .await;
            sqlx::query("ROLLBACK TO SAVEPOINT bad_tokens")
                .execute(&mut ***guard)
                .await
                .map_err(db)?;
            outcomes.push(outcome.is_err());
        }

        Ok(outcomes)
    })
    .await;

    assert_eq!(read.expect("the transaction"), vec![true, true, true]);
}

#[tokio::test]
async fn deleting_a_deployment_takes_its_themes_and_revisions() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresThemeRepository::new(&tx);
        repository
            .insert(&theme(deployment.id, "master", "Autumn", 0))
            .await?;
        repository
            .next_revision(deployment.id, &realm("master"))
            .await?;

        let mut guard = tx.lock().await;
        sqlx::query("DELETE FROM deployments WHERE id = $1")
            .bind(deployment.id.0)
            .execute(&mut ***guard)
            .await
            .map_err(db)?;
        let themes: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM deployment_portal_themes WHERE deployment_id = $1",
        )
        .bind(deployment.id.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(db)?;
        let syncs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM deployment_portal_theme_sync WHERE deployment_id = $1",
        )
        .bind(deployment.id.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(db)?;

        Ok((themes, syncs))
    })
    .await;

    assert_eq!(read.expect("the transaction"), (0, 0));
}

async fn seed(tx: &SharedTx<'_>) -> Result<Deployment, CoreError> {
    let dataplanes = PostgresDataPlaneRepository::new(tx);
    let deployments = PostgresDeploymentRepository::new(tx);

    let dataplane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new("themes-test"),
        Capacity::new(8_000, 16_384, 200).expect("non-zero capacity"),
    );
    dataplanes.save(&dataplane).await?;

    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = UserId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'themes', $3)")
            .bind(user_id.0)
            .bind(format!("{}@themes.test", user_id.0))
            .bind(user_id.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db)?;

        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'themes-test', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
        )
        .bind(organisation_id.0)
        .bind(organisation_id.0.to_string())
        .bind(user_id.0)
        .execute(&mut ***guard)
        .await
        .map_err(db)?;
    }

    let now = Utc::now();
    let deployment = Deployment {
        topology: Default::default(),
        id: DeploymentId(Uuid::new_v4()),
        organisation_id,
        dataplane_id: DataPlaneId(dataplane.id.0),
        name: DeploymentName("themes".to_string()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(0, 5, 0),
        status: DeploymentStatus::Successful,
        namespace: "themes-test".to_string(),
        environment: autharie_domain::deployments::environment::Environment::Development,
        offer: None,
        restored_from: None,
        resources: DeploymentResources::DEFAULT,
        created_by: user_id,
        created_at: now,
        updated_at: now,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: AutoUpgradePolicy::Manual,
        maintenance_window: None,
        network_access: NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    };
    deployments.insert(deployment.clone()).await?;

    Ok(deployment)
}
