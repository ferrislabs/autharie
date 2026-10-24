use autharie_domain::{
    CoreError,
    action::{
        Action, ActionConstraints, ActionFailureReason, ActionId, ActionMetadata, ActionPayload,
        ActionScope, ActionSource, ActionStatus, ActionTarget, ActionType, ActionVersion,
        TargetKind, ports::ActionRepository,
    },
    dataplane::{
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        network::NetworkAccess, ports::DeploymentRepository,
    },
    iam::{
        Envelope, IAM_ACTION_TYPE, IamOperationKind, IamRealm, IamRequest, IamState, IamVerdict,
        ports::IamRepository,
    },
    organisation::OrganisationId,
    upgrades::policy::AutoUpgradePolicy,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    action::PostgresActionRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository, iam::PostgresIamRepository,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

mod support;
use support::pool;

const MAX: usize = 10;

fn db(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

fn envelope_json() -> Value {
    json!({
        "v": 1,
        "alg": "RSA-OAEP-256+A256GCM",
        "ek": URL_SAFE_NO_PAD.encode([1u8; 256]),
        "iv": URL_SAFE_NO_PAD.encode([2u8; 12]),
        "ct": URL_SAFE_NO_PAD.encode([3u8; 64]),
    })
}

fn envelope() -> Envelope {
    Envelope::parse(envelope_json()).unwrap()
}

fn request(deployment: DeploymentId, at: DateTime<Utc>) -> IamRequest {
    IamRequest {
        id: Uuid::new_v4(),
        deployment_id: deployment,
        realm: IamRealm::parse("master").unwrap(),
        operation: IamOperationKind::ApplicationsList,
        actor: "alice".to_string(),
        state: IamState::Pending,
        created_at: at,
        expires_at: at + Duration::minutes(5),
    }
}

async fn stored_envelope(tx: &SharedTx<'_>, id: Uuid) -> Result<Option<Value>, CoreError> {
    let mut guard = tx.lock().await;
    sqlx::query_scalar("SELECT envelope FROM iam_requests WHERE request_id = $1")
        .bind(id)
        .fetch_one(&mut ***guard)
        .await
        .map_err(db)
}

#[tokio::test]
async fn a_request_round_trips_and_is_found_only_under_its_own_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, other) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        let now = Utc::now();
        let asked = request(deployment.id, now);
        assert!(repository.open(&asked, MAX).await?);

        Ok((
            asked.clone(),
            repository.get(deployment.id, asked.id, now).await?,
            repository.get(other.id, asked.id, now).await?,
            repository.get(deployment.id, Uuid::new_v4(), now).await?,
        ))
    })
    .await;

    let (asked, found, elsewhere, unknown) = read.expect("the transaction");
    let found = found.expect("found");
    assert_eq!(found.state, IamState::Pending);
    assert_eq!(found.realm, asked.realm);
    assert_eq!(found.operation, IamOperationKind::ApplicationsList);
    assert_eq!(found.actor, "alice");
    assert_eq!(
        found.expires_at.timestamp_micros(),
        asked.expires_at.timestamp_micros()
    );
    assert!(elsewhere.is_none());
    assert!(unknown.is_none());
}

#[tokio::test]
async fn spec_iam_2_the_stored_value_is_the_envelope_as_received_and_nothing_else() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, _) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        let now = Utc::now();
        let asked = request(deployment.id, now);
        repository.open(&asked, MAX).await?;
        let before = stored_envelope(&tx, asked.id).await?;
        repository
            .record_outcome(deployment.id, asked.id, &IamVerdict::Done(envelope()), now)
            .await?;

        Ok((before, stored_envelope(&tx, asked.id).await?))
    })
    .await;

    let (before, stored) = read.expect("the transaction");
    assert_eq!(before, None);
    assert_eq!(stored, Some(envelope_json()));
}

#[tokio::test]
async fn spec_iam_1_an_envelope_is_taken_once_and_leaves_a_tombstone_without_content() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, other) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        let now = Utc::now();
        let asked = request(deployment.id, now);
        repository.open(&asked, MAX).await?;
        let early = repository
            .take_envelope(deployment.id, asked.id, now)
            .await?;
        repository
            .record_outcome(deployment.id, asked.id, &IamVerdict::Done(envelope()), now)
            .await?;
        let done = repository.get(deployment.id, asked.id, now).await?;
        let elsewhere = repository.take_envelope(other.id, asked.id, now).await?;
        let first = repository
            .take_envelope(deployment.id, asked.id, now)
            .await?;
        let second = repository
            .take_envelope(deployment.id, asked.id, now)
            .await?;
        let tombstone = repository.get(deployment.id, asked.id, now).await?;

        Ok((
            early,
            done,
            elsewhere,
            first,
            second,
            tombstone,
            stored_envelope(&tx, asked.id).await?,
        ))
    })
    .await;

    let (early, done, elsewhere, first, second, tombstone, left) = read.expect("the transaction");
    assert_eq!(early, None);
    assert_eq!(done.expect("found").state, IamState::Done);
    assert_eq!(elsewhere, None);
    assert_eq!(first, Some(envelope()));
    assert_eq!(second, None);
    assert_eq!(tombstone.expect("a tombstone").state, IamState::Delivered);
    assert_eq!(left, None);
}

#[tokio::test]
async fn an_outcome_is_recorded_once_on_a_pending_request_of_its_own_deployment() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, other) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        let now = Utc::now();
        let done = request(deployment.id, now);
        let failed = request(deployment.id, now);
        let delivered = request(deployment.id, now);
        for asked in [&done, &failed, &delivered] {
            repository.open(asked, MAX).await?;
        }
        let failure = IamVerdict::Failed {
            message: "larger than 512 KiB".to_string(),
        };

        let elsewhere = repository
            .record_outcome(other.id, done.id, &IamVerdict::Done(envelope()), now)
            .await?;
        let unknown = repository
            .record_outcome(
                deployment.id,
                Uuid::new_v4(),
                &IamVerdict::Done(envelope()),
                now,
            )
            .await?;
        let first = [
            repository
                .record_outcome(deployment.id, done.id, &IamVerdict::Done(envelope()), now)
                .await?,
            repository
                .record_outcome(deployment.id, failed.id, &failure, now)
                .await?,
            repository
                .record_outcome(
                    deployment.id,
                    delivered.id,
                    &IamVerdict::Done(envelope()),
                    now,
                )
                .await?,
        ];
        repository
            .take_envelope(deployment.id, delivered.id, now)
            .await?;
        let again = [
            repository
                .record_outcome(deployment.id, done.id, &failure, now)
                .await?,
            repository
                .record_outcome(deployment.id, failed.id, &IamVerdict::Done(envelope()), now)
                .await?,
            repository
                .record_outcome(deployment.id, delivered.id, &failure, now)
                .await?,
        ];

        Ok((
            elsewhere,
            unknown,
            first,
            again,
            repository
                .get(deployment.id, done.id, now)
                .await?
                .map(|r| r.state),
            repository
                .get(deployment.id, failed.id, now)
                .await?
                .map(|r| r.state),
            stored_envelope(&tx, failed.id).await?,
        ))
    })
    .await;

    let (elsewhere, unknown, first, again, done, failed, failed_envelope) =
        read.expect("the transaction");
    assert!(!elsewhere);
    assert!(!unknown);
    assert_eq!(first, [true, true, true]);
    assert_eq!(again, [false, false, false]);
    assert_eq!(done, Some(IamState::Done));
    assert_eq!(
        failed,
        Some(IamState::Failed {
            message: "larger than 512 KiB".to_string()
        })
    );
    assert_eq!(failed_envelope, None);
}

#[tokio::test]
async fn spec_iam_9_an_expired_request_is_invisible_and_purged_with_its_envelope() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, _) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        let now = Utc::now();
        let waiting = request(deployment.id, now);
        let tombstone = request(deployment.id, now);
        let fresh = request(deployment.id, now + Duration::minutes(4));
        for asked in [&waiting, &tombstone, &fresh] {
            repository.open(asked, MAX).await?;
        }
        for asked in [&waiting, &tombstone] {
            repository
                .record_outcome(deployment.id, asked.id, &IamVerdict::Done(envelope()), now)
                .await?;
        }
        repository
            .take_envelope(deployment.id, tombstone.id, now)
            .await?;

        let before = now + Duration::minutes(4) + Duration::seconds(59);
        let still_there = repository.get(deployment.id, waiting.id, before).await?;
        let at_expiry = now + Duration::minutes(5);
        let invisible = (
            repository.get(deployment.id, waiting.id, at_expiry).await?,
            repository
                .get(deployment.id, tombstone.id, at_expiry)
                .await?,
            repository
                .take_envelope(deployment.id, waiting.id, at_expiry)
                .await?,
            repository
                .record_outcome(
                    deployment.id,
                    fresh.id,
                    &IamVerdict::Done(envelope()),
                    now + Duration::minutes(9),
                )
                .await?,
        );
        let kept = repository.purge_expired(before).await?;
        let purged = repository.purge_expired(at_expiry).await?;

        Ok((
            still_there,
            invisible,
            kept,
            purged,
            stored_envelope(&tx, fresh.id).await?,
            {
                let mut guard = tx.lock().await;
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM iam_requests WHERE request_id = ANY($1)",
                )
                .bind(vec![waiting.id, tombstone.id])
                .fetch_one(&mut ***guard)
                .await
                .map_err(db)?
            },
        ))
    })
    .await;

    let (still_there, invisible, kept, purged, fresh_envelope, left) =
        read.expect("the transaction");
    assert!(still_there.is_some());
    assert_eq!(invisible, (None, None, None, false));
    assert_eq!(kept, 0);
    assert_eq!(purged, 2);
    assert_eq!(fresh_envelope, None);
    assert_eq!(left, 0);
}

#[tokio::test]
async fn at_most_ten_requests_are_outstanding_per_deployment_and_delivery_or_expiry_frees_a_slot() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, other) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        let now = Utc::now();
        let mut asked = Vec::new();
        for _ in 0..MAX {
            let next = request(deployment.id, now);
            assert!(repository.open(&next, MAX).await?);
            asked.push(next);
        }
        let eleventh = request(deployment.id, now);
        let refused = !repository.open(&eleventh, MAX).await?;
        let refused_row = repository.get(deployment.id, eleventh.id, now).await?;
        let elsewhere = repository.open(&request(other.id, now), MAX).await?;

        repository
            .record_outcome(
                deployment.id,
                asked[0].id,
                &IamVerdict::Done(envelope()),
                now,
            )
            .await?;
        let done_still_counts = !repository.open(&request(deployment.id, now), MAX).await?;
        repository
            .take_envelope(deployment.id, asked[0].id, now)
            .await?;
        let after_delivery = repository.open(&request(deployment.id, now), MAX).await?;
        let full_again = !repository.open(&request(deployment.id, now), MAX).await?;
        let after_expiry = repository
            .open(&request(deployment.id, now + Duration::minutes(6)), MAX)
            .await?;

        Ok((
            refused,
            refused_row,
            elsewhere,
            done_still_counts,
            after_delivery,
            full_again,
            after_expiry,
        ))
    })
    .await;

    let (refused, refused_row, elsewhere, done_still_counts, after_delivery, full_again, expired) =
        read.expect("the transaction");
    assert!(refused);
    assert!(refused_row.is_none());
    assert!(elsewhere);
    assert!(done_still_counts);
    assert!(after_delivery);
    assert!(full_again);
    assert!(expired);
}

#[tokio::test]
async fn deleting_a_deployment_deletes_its_requests() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, _) = seed(&tx).await?;
        let repository = PostgresIamRepository::new(&tx);
        repository
            .open(&request(deployment.id, Utc::now()), MAX)
            .await?;

        let mut guard = tx.lock().await;
        sqlx::query("DELETE FROM deployments WHERE id = $1")
            .bind(deployment.id.0)
            .execute(&mut ***guard)
            .await
            .map_err(db)?;
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM iam_requests WHERE deployment_id = $1")
            .bind(deployment.id.0)
            .fetch_one(&mut ***guard)
            .await
            .map_err(db)
    })
    .await;

    assert_eq!(read.expect("the transaction"), 0);
}

fn iam_payload() -> Value {
    json!({
        "realm": "master",
        "request_id": Uuid::nil(),
        "operation": { "type": "applications.list", "page": 1, "limit": 25, "search": "alice-canary" },
        "sealing_key": { "kty": "RSA", "n": "AQ", "e": "AQAB", "alg": "RSA-OAEP-256" }
    })
}

fn redacted_payload() -> Value {
    json!({
        "realm": "master",
        "request_id": Uuid::nil(),
        "operation": { "type": "applications.list" },
        "sealing_key": { "kty": "RSA", "n": "AQ", "e": "AQAB", "alg": "RSA-OAEP-256" }
    })
}

#[tokio::test]
async fn publishing_or_failing_an_iam_action_drops_the_search_from_the_stored_payload() {
    let Some(pool) = pool().await else {
        return;
    };

    let read: Result<_, CoreError> = in_scratch_tx(&pool, db, async |tx| {
        let (deployment, _) = seed(&tx).await?;
        let actions = PostgresActionRepository::new(&tx);
        let published = leased(&deployment, IAM_ACTION_TYPE, iam_payload());
        let failing = leased(&deployment, IAM_ACTION_TYPE, iam_payload());
        let get = leased(
            &deployment,
            IAM_ACTION_TYPE,
            json!({ "operation": { "type": "applications.get", "id": Uuid::nil() } }),
        );
        let other = leased(&deployment, "deployment.smtp", iam_payload());
        for action in [&published, &failing, &get, &other] {
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
            .find(|action| action.id == published.id)
            .map(|action| action.payload.data.clone());

        let scope = ActionScope::Deployment(deployment.id);
        actions
            .ack_published(scope, published.id, Utc::now())
            .await?;
        actions.ack_published(scope, get.id, Utc::now()).await?;
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
            read(published.id).await?.expect("stored").payload.data,
            read(failing.id).await?.expect("stored").payload.data,
            read(get.id).await?.expect("stored").payload.data,
            read(other.id).await?.expect("stored").payload.data,
        ))
    })
    .await;

    let (before_ack, published, failed, get, untouched) = read.expect("the transaction");
    assert_eq!(
        before_ack.expect("claimed")["operation"]["search"],
        "alice-canary"
    );
    assert_eq!(published, redacted_payload());
    assert_eq!(failed, redacted_payload());
    assert_eq!(get, json!({ "operation": { "type": "applications.get" } }));
    assert_eq!(untouched["operation"]["search"], "alice-canary");
}

fn leased(deployment: &Deployment, action_type: &str, data: Value) -> Action {
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

fn deployment_of(
    organisation_id: OrganisationId,
    dataplane: DataPlaneId,
    user_id: UserId,
    name: &str,
) -> Deployment {
    let now = Utc::now();

    Deployment {
        topology: Default::default(),
        id: DeploymentId(Uuid::new_v4()),
        organisation_id,
        dataplane_id: dataplane,
        name: DeploymentName(name.to_string()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(0, 5, 0),
        status: DeploymentStatus::Successful,
        namespace: format!("iam-test-{name}"),
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
    }
}

async fn seed(tx: &SharedTx<'_>) -> Result<(Deployment, Deployment), CoreError> {
    let dataplanes = PostgresDataPlaneRepository::new(tx);
    let deployments = PostgresDeploymentRepository::new(tx);

    let dataplane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new("iam-test"),
        Capacity::new(8_000, 16_384, 200).expect("non-zero capacity"),
    );
    dataplanes.save(&dataplane).await?;

    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = UserId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'iam', $3)")
            .bind(user_id.0)
            .bind(format!("{}@iam.test", user_id.0))
            .bind(user_id.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db)?;

        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'iam-test', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
        )
        .bind(organisation_id.0)
        .bind(organisation_id.0.to_string())
        .bind(user_id.0)
        .execute(&mut ***guard)
        .await
        .map_err(db)?;
    }

    let first = deployment_of(organisation_id, dataplane.id, user_id, "first");
    let second = deployment_of(organisation_id, dataplane.id, user_id, "second");
    deployments.insert(first.clone()).await?;
    deployments.insert(second.clone()).await?;

    Ok((first, second))
}
