use autharie_domain::{
    CoreError,
    action::{
        Action, ActionConstraints, ActionId, ActionMetadata, ActionPayload, ActionScope,
        ActionSource, ActionStatus, ActionTarget, ActionType, ActionVersion, TargetKind,
        ports::ActionRepository,
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
    upgrades::policy::AutoUpgradePolicy,
    user::UserId,
    version::Version,
    webhooks::{
        HeaderName, RequestKind, RequestPhase, SecretRecord, StoredWebhook, Webhook,
        WebhookEndpoint, WebhookEvent, WebhookId, WebhookName, WebhookRealm, WebhookRequest,
        WebhookState, ports::WebhookRepository,
    },
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    action::PostgresActionRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository, webhooks::PostgresWebhookRepository,
};
use chrono::{DateTime, Duration, Utc};
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

fn webhook(
    deployment: DeploymentId,
    name: &str,
    revision: u64,
    at: DateTime<Utc>,
) -> StoredWebhook {
    StoredWebhook {
        webhook: Webhook {
            id: WebhookId(Uuid::new_v4()),
            deployment_id: deployment,
            realm: WebhookRealm::master(),
            name: WebhookName::parse(name).unwrap(),
            description: Some("what for".to_string()),
            endpoint: WebhookEndpoint::parse("https://example.com/hook").unwrap(),
            events: vec![
                WebhookEvent::parse("user.created").unwrap(),
                WebhookEvent::parse("realm.deleted").unwrap(),
            ],
            headers: vec![HeaderName::parse("x-api-key").unwrap()],
            state: WebhookState::Pending,
            secret_version: None,
            secret_received_at: None,
            desired_revision: revision,
            created_at: at,
            updated_at: at,
        },
        sealed_headers: Some(sealed(7)),
    }
}

#[tokio::test]
async fn a_webhook_survives_the_round_trip_with_its_sealed_headers() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let stored = webhook(deployment.id, "orders", 1, Utc::now());

        repository.insert(&stored).await?;

        Ok((
            stored.clone(),
            repository.get(deployment.id, stored.webhook.id).await?,
            repository.list(deployment.id).await?,
        ))
    })
    .await;

    let (stored, got, listed) = read.expect("the transaction");
    let got = got.expect("found");
    assert_eq!(got.sealed_headers, stored.sealed_headers);
    assert_eq!(got.webhook.events, stored.webhook.events);
    assert_eq!(got.webhook.headers, stored.webhook.headers);
    assert_eq!(got.webhook.name, stored.webhook.name);
    assert_eq!(got.webhook.endpoint, stored.webhook.endpoint);
    assert_eq!(got.webhook.desired_revision, 1);
    assert_eq!(got.webhook.state, WebhookState::Pending);
    assert_eq!(listed.len(), 1);
}

#[tokio::test]
async fn a_webhook_is_only_found_under_its_own_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let stored = webhook(deployment.id, "orders", 1, Utc::now());
        repository.insert(&stored).await?;

        Ok((
            repository
                .get(DeploymentId(Uuid::new_v4()), stored.webhook.id)
                .await?,
            repository
                .delete(DeploymentId(Uuid::new_v4()), stored.webhook.id)
                .await?,
        ))
    })
    .await;

    let (found, deleted) = read.expect("the transaction");
    assert!(found.is_none());
    assert!(!deleted);
}

#[tokio::test]
async fn an_update_replaces_the_fields_and_the_sealed_headers() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let mut stored = webhook(deployment.id, "orders", 1, Utc::now());
        repository.insert(&stored).await?;

        stored.webhook.name = WebhookName::parse("renamed").unwrap();
        stored.webhook.headers = vec![];
        stored.sealed_headers = None;
        stored.webhook.desired_revision = 2;
        stored.webhook.state = WebhookState::Failed {
            reason: "ssrf".to_string(),
        };
        let updated = repository.update(&stored).await?;

        Ok((
            updated,
            repository.get(deployment.id, stored.webhook.id).await?,
        ))
    })
    .await;

    let (updated, got) = read.expect("the transaction");
    let got = got.expect("found");
    assert!(updated);
    assert_eq!(got.webhook.name.as_str(), "renamed");
    assert!(got.webhook.headers.is_empty());
    assert_eq!(got.sealed_headers, None);
    assert_eq!(got.webhook.desired_revision, 2);
    assert_eq!(
        got.webhook.state,
        WebhookState::Failed {
            reason: "ssrf".to_string()
        }
    );
}

#[tokio::test]
async fn the_revision_counts_up_per_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let before = repository.sync_state(deployment.id).await?;

        let first = repository.next_revision(deployment.id).await?;
        let second = repository.next_revision(deployment.id).await?;

        Ok((
            before,
            first,
            second,
            repository.sync_state(deployment.id).await?,
        ))
    })
    .await;

    let (before, first, second, after) = read.expect("the transaction");
    assert_eq!(
        (before.revision, before.applied_revision, before.realms),
        (0, 0, None)
    );
    assert_eq!((first, second), (1, 2));
    assert_eq!(after.revision, 2);
}

#[tokio::test]
async fn the_applied_revision_never_goes_back_and_realms_are_kept() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let realms = vec!["master".to_string(), "customers".to_string()];

        repository
            .record_applied(deployment.id, 5, &realms, Utc::now())
            .await?;
        repository
            .record_applied(deployment.id, 3, &realms, Utc::now())
            .await?;
        let after_old = repository.sync_state(deployment.id).await?;

        repository
            .record_applied(deployment.id, 6, &[], Utc::now())
            .await?;
        let after_empty = repository.sync_state(deployment.id).await?;

        Ok((after_old, after_empty))
    })
    .await;

    let (after_old, after_empty) = read.expect("the transaction");
    assert_eq!(after_old.applied_revision, 5);
    assert_eq!(
        after_old.realms,
        Some(vec!["master".to_string(), "customers".to_string()])
    );
    assert_eq!(after_empty.applied_revision, 6);
    assert_eq!(after_empty.realms, None);
}

#[tokio::test]
async fn a_secret_only_moves_forward_and_shows_on_the_webhook() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let stored = webhook(deployment.id, "orders", 1, Utc::now());
        let id = stored.webhook.id;
        repository.insert(&stored).await?;
        let now = Utc::now();

        let none = repository.secret(id).await?;
        let first = repository
            .store_secret(
                id,
                &SecretRecord {
                    version: 1,
                    sealed: sealed(1),
                    received_at: now,
                },
            )
            .await?;
        let replay = repository
            .store_secret(
                id,
                &SecretRecord {
                    version: 1,
                    sealed: sealed(9),
                    received_at: now,
                },
            )
            .await?;
        let older_after_newer = {
            repository
                .store_secret(
                    id,
                    &SecretRecord {
                        version: 3,
                        sealed: sealed(3),
                        received_at: now,
                    },
                )
                .await?;
            repository
                .store_secret(
                    id,
                    &SecretRecord {
                        version: 2,
                        sealed: sealed(2),
                        received_at: now,
                    },
                )
                .await?
        };

        Ok((
            none,
            first,
            replay,
            older_after_newer,
            repository.secret(id).await?,
            repository.get(deployment.id, id).await?,
        ))
    })
    .await;

    let (none, first, replay, older, current, shown) = read.expect("the transaction");
    assert!(none.is_none());
    assert!(first);
    assert!(!replay);
    assert!(!older);
    let current = current.expect("a secret");
    assert_eq!(current.version, 3);
    assert_eq!(current.sealed, sealed(3));
    assert_eq!(shown.expect("found").webhook.secret_version, Some(3));
}

#[tokio::test]
async fn deleting_a_webhook_takes_its_secret_and_requests_with_it() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let stored = webhook(deployment.id, "orders", 1, Utc::now());
        let id = stored.webhook.id;
        repository.insert(&stored).await?;
        repository
            .store_secret(
                id,
                &SecretRecord {
                    version: 1,
                    sealed: sealed(1),
                    received_at: Utc::now(),
                },
            )
            .await?;
        repository
            .insert_request(&request(id, RequestKind::RotateSecret, Utc::now()))
            .await?;

        let deleted = repository.delete(deployment.id, id).await?;

        Ok((
            deleted,
            repository.secret(id).await?,
            repository.recent_requests(deployment.id, 10).await?,
        ))
    })
    .await;

    let (deleted, secret, requests) = read.expect("the transaction");
    assert!(deleted);
    assert!(secret.is_none());
    assert!(requests.is_empty());
}

fn request(webhook: WebhookId, kind: RequestKind, at: DateTime<Utc>) -> WebhookRequest {
    WebhookRequest {
        id: Uuid::new_v4(),
        webhook_id: webhook,
        kind,
        delivery_id: (kind == RequestKind::RetryDelivery).then(|| "delivery-1".to_string()),
        phase: RequestPhase::Pending,
        message: None,
        created_at: at,
        updated_at: at,
    }
}

#[tokio::test]
async fn a_request_is_finished_once_and_only_the_latest_are_listed() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let repository = PostgresWebhookRepository::new(&tx);
        let stored = webhook(deployment.id, "orders", 1, Utc::now());
        let id = stored.webhook.id;
        repository.insert(&stored).await?;
        let start = Utc::now();
        let mut ids = vec![];
        for offset in 0..4 {
            let pending = request(
                id,
                RequestKind::RetryDelivery,
                start + Duration::seconds(offset),
            );
            ids.push(pending.id);
            repository.insert_request(&pending).await?;
        }

        let first = repository
            .finish_request(ids[0], RequestPhase::Failed, Some("nope"), Utc::now())
            .await?;
        let again = repository
            .finish_request(ids[0], RequestPhase::Done, None, Utc::now())
            .await?;
        let back_to_pending = repository
            .finish_request(ids[1], RequestPhase::Pending, None, Utc::now())
            .await?;
        let unknown = repository
            .finish_request(Uuid::new_v4(), RequestPhase::Done, None, Utc::now())
            .await?;
        let latest = repository.recent_requests(deployment.id, 2).await?;

        Ok((ids, first, again, back_to_pending, unknown, latest))
    })
    .await;

    let (ids, first, again, back_to_pending, unknown, latest) = read.expect("the transaction");
    assert!(first);
    assert!(!again);
    assert!(!back_to_pending);
    assert!(!unknown);
    assert_eq!(
        latest.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![ids[3], ids[2]]
    );
    assert_eq!(latest[0].kind, RequestKind::RetryDelivery);
    assert_eq!(latest[0].delivery_id.as_deref(), Some("delivery-1"));
}

#[tokio::test]
async fn publishing_a_webhooks_action_drops_the_header_values_from_the_stored_payload() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let deployment = seed(&tx).await?;
        let actions = PostgresActionRepository::new(&tx);
        let webhooks = leased(&deployment, "deployment.webhooks");
        let other = leased(&deployment, "deployment.iam_settings");
        let failing = leased(&deployment, "deployment.webhooks");
        for action in [&webhooks, &other, &failing] {
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
            .find(|action| action.id == webhooks.id)
            .map(|action| action.payload.data.clone());

        let scope = ActionScope::Deployment(deployment.id);
        actions
            .ack_published(scope, webhooks.id, Utc::now())
            .await?;
        actions.ack_published(scope, other.id, Utc::now()).await?;
        actions
            .ack_failed(
                scope,
                failing.id,
                autharie_domain::action::ActionFailureReason::PublishFailed,
                Utc::now(),
            )
            .await?;

        let read = |id: ActionId| {
            let actions = &actions;
            async move { actions.get_by_id(deployment.id, id).await }
        };

        Ok((
            before_ack,
            read(webhooks.id).await?.expect("stored").payload.data,
            read(other.id).await?.expect("stored").payload.data,
            read(failing.id).await?.expect("stored").payload.data,
        ))
    })
    .await;

    let (before_ack, published, untouched, failed) = read.expect("the transaction");
    assert!(before_ack.expect("claimed").get("headers").is_some());
    assert_eq!(published, json!({ "revision": 1, "webhooks": [] }));
    assert!(untouched.get("headers").is_some());
    assert_eq!(failed, json!({ "revision": 1, "webhooks": [] }));
}

fn leased(deployment: &Deployment, action_type: &str) -> Action {
    Action {
        id: ActionId(Uuid::new_v4()),
        deployment_id: Some(deployment.id),
        dataplane_id: deployment.dataplane_id,
        action_type: ActionType(action_type.to_string()),
        target: ActionTarget {
            kind: TargetKind::Deployment,
            id: deployment.id.0,
        },
        payload: ActionPayload {
            data: json!({
                "revision": 1,
                "webhooks": [],
                "headers": { "w": { "x-api-key": "hunter2" } }
            }),
        },
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
        Region::new("webhooks-test"),
        Capacity::new(8_000, 16_384, 200).expect("non-zero capacity"),
    );
    dataplanes.save(&dataplane).await?;

    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = UserId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'webhooks', $3)")
            .bind(user_id.0)
            .bind(format!("{}@webhooks.test", user_id.0))
            .bind(user_id.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db)?;

        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'webhooks-test', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
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
        name: DeploymentName("webhooks".to_string()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(0, 5, 0),
        status: DeploymentStatus::Successful,
        namespace: "webhooks-test".to_string(),
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
