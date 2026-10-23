use autharie_domain::{
    CoreError,
    action::{
        Action, ActionConstraints, ActionFailureReason, ActionId, ActionMetadata, ActionPayload,
        ActionScope, ActionSource, ActionStatus, ActionTarget, ActionType, ActionVersion,
        TargetKind, ports::ActionRepository,
    },
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    dataplane::{
        credential_repository::SealedSecret,
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        network::NetworkAccess, ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    smtp::{
        FromEmail, FromName, SmtpConfig, SmtpEncryption, SmtpHost, SmtpPort, SmtpRealm,
        SmtpSettings, SmtpState, SmtpUsername, StoredSmtp, ports::SmtpRepository,
    },
    upgrades::policy::AutoUpgradePolicy,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    action::PostgresActionRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository, smtp::PostgresSmtpRepository,
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

fn sealed(tag: u8) -> SealedSecret {
    SealedSecret {
        ciphertext: vec![tag; 24],
        nonce: vec![tag; 12],
        wrapped_dek: WrappedDek::new(format!("vault:v1:{tag}")),
        key: KeyRef::new(
            ProviderName::platform(),
            KeyName::new("cloud-credentials").unwrap(),
            KeyVersion::new(1),
        ),
    }
}

fn realm(name: &str) -> SmtpRealm {
    SmtpRealm::parse(name).unwrap()
}

fn stored(deployment: DeploymentId, realm_name: &str, host: &str, tag: u8) -> StoredSmtp {
    StoredSmtp {
        config: SmtpConfig {
            deployment_id: deployment,
            realm: realm(realm_name),
            settings: SmtpSettings {
                host: SmtpHost::parse(host).unwrap(),
                port: SmtpPort::parse(587).unwrap(),
                encryption: SmtpEncryption::Starttls,
                username: SmtpUsername::parse("apikey").unwrap(),
                from_email: FromEmail::parse("no-reply@example.com").unwrap(),
                from_name: FromName::parse("Acme").unwrap(),
            },
            state: SmtpState::Pending,
            updated_at: Utc::now(),
        },
        sealed_password: sealed(tag),
    }
}

#[tokio::test]
async fn a_config_survives_the_round_trip_with_its_sealed_password() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresSmtpRepository::new(&tx);
        let config = stored(deployment.id, "master", "smtp.example.com", 7);

        repository.upsert(&config).await?;

        Ok((
            config.clone(),
            repository.get(deployment.id, &realm("master")).await?,
        ))
    })
    .await;

    let (config, got) = read.expect("the transaction");
    let got = got.expect("found");
    assert_eq!(got.sealed_password, config.sealed_password);
    assert_eq!(got.config.settings, config.config.settings);
    assert_eq!(got.config.state, SmtpState::Pending);
    assert_eq!(got.config.realm, realm("master"));
}

#[tokio::test]
async fn a_config_is_found_only_under_its_own_deployment_and_realm() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresSmtpRepository::new(&tx);
        repository
            .upsert(&stored(deployment.id, "master", "smtp.example.com", 7))
            .await?;

        Ok((
            repository
                .get(DeploymentId(Uuid::new_v4()), &realm("master"))
                .await?,
            repository.get(deployment.id, &realm("customers")).await?,
            repository
                .delete(DeploymentId(Uuid::new_v4()), &realm("master"))
                .await?,
            repository
                .delete(deployment.id, &realm("customers"))
                .await?,
        ))
    })
    .await;

    let (other_deployment, other_realm, deleted_elsewhere, deleted_other_realm) =
        read.expect("the transaction");
    assert!(other_deployment.is_none());
    assert!(other_realm.is_none());
    assert!(!deleted_elsewhere);
    assert!(!deleted_other_realm);
}

#[tokio::test]
async fn an_upsert_replaces_the_row_of_the_realm_and_leaves_the_others() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresSmtpRepository::new(&tx);
        repository
            .upsert(&stored(deployment.id, "master", "smtp.example.com", 1))
            .await?;
        repository
            .upsert(&stored(
                deployment.id,
                "customers",
                "smtp.customers.test",
                2,
            ))
            .await?;

        let mut changed = stored(deployment.id, "master", "relay.example.com", 3);
        changed.config.state = SmtpState::Applied;
        repository.upsert(&changed).await?;

        Ok((
            repository.get(deployment.id, &realm("master")).await?,
            repository.get(deployment.id, &realm("customers")).await?,
        ))
    })
    .await;

    let (master, customers) = read.expect("the transaction");
    let master = master.expect("found");
    assert_eq!(master.config.settings.host.as_str(), "relay.example.com");
    assert_eq!(master.sealed_password, sealed(3));
    assert_eq!(master.config.state, SmtpState::Applied);
    let customers = customers.expect("found");
    assert_eq!(
        customers.config.settings.host.as_str(),
        "smtp.customers.test"
    );
    assert_eq!(customers.sealed_password, sealed(2));
}

#[tokio::test]
async fn a_state_is_recorded_with_its_message() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresSmtpRepository::new(&tx);
        repository
            .upsert(&stored(deployment.id, "master", "smtp.example.com", 1))
            .await?;
        let failed = SmtpState::Failed {
            message: "535 authentication failed".to_string(),
        };

        let set = repository
            .set_state(deployment.id, &realm("master"), &failed, Utc::now())
            .await?;
        let failed_row = repository.get(deployment.id, &realm("master")).await?;
        repository
            .set_state(
                deployment.id,
                &realm("master"),
                &SmtpState::Applied,
                Utc::now(),
            )
            .await?;
        let applied_row = repository.get(deployment.id, &realm("master")).await?;
        let missing = repository
            .set_state(
                deployment.id,
                &realm("customers"),
                &SmtpState::Applied,
                Utc::now(),
            )
            .await?;

        Ok((set, failed_row, applied_row, missing))
    })
    .await;

    let (set, failed_row, applied_row, missing) = read.expect("the transaction");
    assert!(set);
    assert_eq!(
        failed_row.expect("found").config.state,
        SmtpState::Failed {
            message: "535 authentication failed".to_string()
        }
    );
    assert_eq!(applied_row.expect("found").config.state, SmtpState::Applied);
    assert!(!missing);
}

#[tokio::test]
async fn the_revision_counts_up_per_realm_and_never_goes_back_after_a_delete() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresSmtpRepository::new(&tx);
        let master = realm("master");
        let customers = realm("customers");

        let before = repository.revision(deployment.id, &master).await?;
        repository
            .upsert(&stored(deployment.id, "master", "smtp.example.com", 1))
            .await?;
        let first = repository.next_revision(deployment.id, &master).await?;
        let second = repository.next_revision(deployment.id, &master).await?;
        let other_realm = repository.next_revision(deployment.id, &customers).await?;

        let deleted = repository.delete(deployment.id, &master).await?;
        let after_delete = repository.revision(deployment.id, &master).await?;
        repository
            .upsert(&stored(deployment.id, "master", "smtp.example.com", 1))
            .await?;
        let recreated = repository.next_revision(deployment.id, &master).await?;

        Ok((
            before,
            first,
            second,
            other_realm,
            deleted,
            after_delete,
            recreated,
        ))
    })
    .await;

    let (before, first, second, other_realm, deleted, after_delete, recreated) =
        read.expect("the transaction");
    assert_eq!(before, 0);
    assert_eq!((first, second), (1, 2));
    assert_eq!(other_realm, 1);
    assert!(deleted);
    assert_eq!(after_delete, 2);
    assert_eq!(recreated, 3);
}

#[tokio::test]
async fn deleting_a_deployment_takes_its_configs_and_revisions() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresSmtpRepository::new(&tx);
        repository
            .upsert(&stored(deployment.id, "master", "smtp.example.com", 1))
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
        let configs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM deployment_smtp_configs WHERE deployment_id = $1",
        )
        .bind(deployment.id.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(db)?;
        let syncs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM deployment_smtp_sync WHERE deployment_id = $1",
        )
        .bind(deployment.id.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(db)?;

        Ok((configs, syncs))
    })
    .await;

    assert_eq!(read.expect("the transaction"), (0, 0));
}

#[tokio::test]
async fn publishing_or_failing_an_smtp_action_drops_the_password_from_the_stored_payload() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let actions = PostgresActionRepository::new(&tx);
        let saved = leased(&deployment, "deployment.smtp", save_payload());
        let failing = leased(&deployment, "deployment.smtp", save_payload());
        let removal = leased(
            &deployment,
            "deployment.smtp",
            json!({ "revision": 3, "realm": "master", "smtp": null }),
        );
        let other = leased(&deployment, "deployment.iam_settings", save_payload());
        for action in [&saved, &failing, &removal, &other] {
            actions.append(action.clone()).await?;
        }

        let claimed = actions
            .claim_pending(
                deployment.dataplane_id,
                vec![deployment.id],
                10,
                Utc::now(),
                Utc::now() + Duration::minutes(5),
            )
            .await?;
        let before_ack = claimed
            .iter()
            .find(|action| action.id == saved.id)
            .map(|action| action.payload.data.clone());

        let scope = ActionScope::Deployment(deployment.id);
        actions.ack_published(scope, saved.id, Utc::now()).await?;
        actions.ack_published(scope, removal.id, Utc::now()).await?;
        actions.ack_published(scope, other.id, Utc::now()).await?;
        actions
            .ack_failed(
                scope,
                failing.id,
                ActionFailureReason::PublishFailed,
                Utc::now(),
            )
            .await?;

        let read = |id: ActionId| {
            let actions = &actions;
            async move { actions.get_by_id(deployment.id, id).await }
        };

        Ok((
            before_ack,
            read(saved.id).await?.expect("stored").payload.data,
            read(failing.id).await?.expect("stored").payload.data,
            read(removal.id).await?.expect("stored").payload.data,
            read(other.id).await?.expect("stored").payload.data,
        ))
    })
    .await;

    let (before_ack, published, failed, removal, untouched) = read.expect("the transaction");
    assert_eq!(
        before_ack.expect("claimed")["smtp"]["password"],
        "hunter2-canary"
    );
    let without_password = json!({
        "revision": 1,
        "realm": "master",
        "smtp": { "host": "smtp.example.com", "port": 587 }
    });
    assert_eq!(published, without_password);
    assert_eq!(failed, without_password);
    assert_eq!(
        removal,
        json!({ "revision": 3, "realm": "master", "smtp": null })
    );
    assert_eq!(untouched["smtp"]["password"], "hunter2-canary");
}

fn save_payload() -> serde_json::Value {
    json!({
        "revision": 1,
        "realm": "master",
        "smtp": { "host": "smtp.example.com", "port": 587, "password": "hunter2-canary" }
    })
}

fn leased(deployment: &Deployment, action_type: &str, data: serde_json::Value) -> Action {
    Action {
        id: ActionId(Uuid::new_v4()),
        deployment_id: Some(deployment.id),
        dataplane_id: deployment.dataplane_id,
        action_type: ActionType(action_type.to_string()),
        target: ActionTarget {
            kind: TargetKind::Deployment,
            id: deployment.id.0,
        },
        payload: ActionPayload { data },
        version: ActionVersion(1),
        status: ActionStatus::Pending,
        metadata: ActionMetadata {
            source: ActionSource::System,
            created_at: Utc::now(),
            constraints: ActionConstraints::default(),
        },
        leased_until: None,
    }
}

async fn seed(tx: &SharedTx<'_>) -> Result<Deployment, CoreError> {
    let dataplanes = PostgresDataPlaneRepository::new(tx);
    let deployments = PostgresDeploymentRepository::new(tx);

    let dataplane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new("smtp-test"),
        Capacity::new(8_000, 16_384, 200).expect("non-zero capacity"),
    );
    dataplanes.save(&dataplane).await?;

    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = UserId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'smtp', $3)")
            .bind(user_id.0)
            .bind(format!("{}@smtp.test", user_id.0))
            .bind(user_id.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db)?;

        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'smtp-test', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
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
        name: DeploymentName("smtp".to_string()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(0, 5, 0),
        status: DeploymentStatus::Successful,
        namespace: "smtp-test".to_string(),
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
