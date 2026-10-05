use std::collections::HashSet;

use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    dataplane::{
        cloud_provider::{
            ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeType, Provider,
        },
        cluster_profile::{ClusterMode, ClusterProfile, Replication},
        credential::{CloudCredential, CloudCredentialId, ScopeCheck},
        credential_repository::{
            CloudCredentialRepository, SealedCredential, SealedSecret, SealedSecretRepository,
        },
        entities::DataPlane,
        herald_identity::HeraldBinding,
        inventory::{ClusterInventory, ProvisionedResource, ResourceKind},
        ports::DataPlaneRepository,
        value_objects::{
            Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneStatus, DeploymentResources,
            Region,
        },
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        distribution::Distribution, ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, with_tx};
use autharie_postgres::{
    credentials::{PostgresCloudCredentialRepository, PostgresSealedSecretRepository},
    dataplane::{PostgresClusterClaims, PostgresClusterInventory, PostgresDataPlaneRepository},
    deployments::PostgresDeploymentRepository,
};
use chrono::Utc;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::Mutex;
use uuid::Uuid;

const REGION: &str = "cluster-claims-test";

static COMMITTED: Mutex<()> = Mutex::const_new(());

fn db_error(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());

    let Some(url) = url else {
        assert!(
            std::env::var("REQUIRE_DATABASE_URL").is_err(),
            "REQUIRE_DATABASE_URL is set but DATABASE_URL is not"
        );
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");
    wipe(&pool).await;
    Some(pool)
}

async fn wipe(pool: &PgPool) {
    for statement in [
        "DELETE FROM cluster_inventory WHERE data_plane_id IN \
         (SELECT id FROM data_planes WHERE region = $1)",
        "DELETE FROM deployments WHERE dataplane_id IN \
         (SELECT id FROM data_planes WHERE region = $1)",
        "DELETE FROM data_planes WHERE region = $1",
    ] {
        sqlx::query(statement)
            .bind(REGION)
            .execute(pool)
            .await
            .expect("the test data was removed");
    }
}

fn profile() -> ClusterProfile {
    ClusterProfile::restore(
        ClusterMode::Standard,
        ControlPlaneOffer {
            id: ControlPlaneOfferId::new("kapsule-dedicated-4"),
            kind: ControlPlaneKind::Dedicated,
            monthly_price: Money::new(7_000),
        },
        NodeType::new("PRO2-S"),
        2,
        6,
        Replication::new(2).expect("replicas"),
    )
    .expect("a valid profile")
}

struct Seeded {
    user: UserId,
    organisation: OrganisationId,
    credential: CloudCredentialId,
}

async fn seed_account(pool: &PgPool) -> Seeded {
    with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        let user = UserId(Uuid::new_v4());
        let organisation = OrganisationId(Uuid::new_v4());
        let credential = CloudCredentialId(Uuid::new_v4());
        {
            let mut guard = tx.lock().await;
            sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'claims', $3)")
                .bind(user.0)
                .bind(format!("{}@cluster-claims.test", user.0))
                .bind(user.0.to_string())
                .execute(&mut ***guard)
                .await
                .map_err(db_error)?;
            sqlx::query(
                "INSERT INTO organisations \
                 (id, name, slug, owner_id, status, plan, max_instances, max_users, \
                  max_storage_gb, created_at, updated_at) \
                 VALUES ($1, 'claims', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
            )
            .bind(organisation.0)
            .bind(format!("claims-{}", organisation.0))
            .bind(user.0)
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        }
        PostgresSealedSecretRepository::new(&tx)
            .insert(&SealedCredential {
                id: credential,
                organisation_id: organisation,
                provider: Provider::Scaleway,
                sealed: SealedSecret {
                    ciphertext: vec![1, 2, 3, 4],
                    nonce: vec![9; 12],
                    wrapped_dek: WrappedDek::new("vault:v1:opaque"),
                    key: KeyRef::new(
                        ProviderName::platform(),
                        KeyName::new("cloud-credentials").expect("a key name"),
                        KeyVersion::new(1),
                    ),
                },
            })
            .await?;
        PostgresCloudCredentialRepository::new(&tx)
            .insert(&CloudCredential {
                id: credential,
                organisation_id: organisation,
                provider: Provider::Scaleway,
                label: "claims".to_string(),
                scope_check: ScopeCheck {
                    checked_at: Utc::now(),
                },
                created_at: Utc::now(),
            })
            .await?;
        Ok(Seeded {
            user,
            organisation,
            credential,
        })
    })
    .await
    .expect("the account was seeded")
}

async fn seed_customer_plane(pool: &PgPool, account: &Seeded) -> (DataPlaneId, DeploymentId) {
    with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        let deployment_id = DeploymentId(Uuid::new_v4());
        let plane = DataPlane::new(
            DataPlaneAllocation::Customer {
                organisation_id: account.organisation,
                deployment_id,
                credential_id: account.credential,
            },
            Region::new(REGION),
            Capacity::new(1, 1, 1).expect("non-zero capacity"),
        );
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;

        let at = Utc::now();
        PostgresDeploymentRepository::new(&tx)
            .insert(Deployment {
                id: deployment_id,
                organisation_id: account.organisation,
                dataplane_id: plane.id,
                name: DeploymentName(format!("claims-{}", deployment_id.0)),
                kind: DeploymentKind::Ferriskey,
                version: Version::new(26, 0, 1),
                status: DeploymentStatus::Pending,
                namespace: format!("claims-{}", deployment_id.0),
                environment: autharie_domain::deployments::environment::Environment::Development,
                offer: None,
                restored_from: None,
                resources: DeploymentResources::DEFAULT,
                created_by: account.user,
                created_at: at,
                updated_at: at,
                deployed_at: None,
                deleted_at: None,
                auto_upgrade: Default::default(),
                maintenance_window: None,
                network_access: autharie_domain::deployments::network::NetworkAccess::Open,
                last_verified_restore_at: None,
                last_restore_drill_seconds: None,
                log_shipping_enabled: false,
                iam_settings: Default::default(),
                distribution: Distribution::CustomerCloud {
                    credential_id: account.credential,
                    profile: profile(),
                },
            })
            .await?;
        Ok((plane.id, deployment_id))
    })
    .await
    .expect("the plane was seeded")
}

async fn mine(pool: &PgPool, ids: Vec<DataPlaneId>) -> Vec<DataPlaneId> {
    let raw: Vec<Uuid> = ids.iter().map(|id| id.0).collect();
    let owned: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM data_planes WHERE id = ANY($1) AND region = $2")
            .bind(&raw)
            .bind(REGION)
            .fetch_all(pool)
            .await
            .expect("the ownership read ran");
    ids.into_iter().filter(|id| owned.contains(&id.0)).collect()
}

async fn claim(pool: &PgPool, lease_seconds: f64, limit: i64) -> Vec<DataPlaneId> {
    let claimed = with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        PostgresClusterClaims::new(&tx)
            .claim_provisioning(lease_seconds, limit)
            .await
    })
    .await
    .expect("the claim ran");
    mine(pool, claimed).await
}

async fn candidates(pool: &PgPool) -> Vec<DataPlaneId> {
    let listed = with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        PostgresClusterClaims::new(&tx)
            .teardown_candidates(1000)
            .await
    })
    .await
    .expect("the listing ran");
    mine(pool, listed).await
}

async fn read(pool: &PgPool, id: DataPlaneId) -> DataPlane {
    with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        PostgresDataPlaneRepository::new(&tx).find_by_id(&id).await
    })
    .await
    .expect("the read ran")
    .expect("the plane exists")
}

fn built(plane: &DataPlane) -> DataPlane {
    let mut built = plane.clone();
    built.herald = Some(HeraldBinding {
        client_id: format!("herald-{}", plane.id.0),
        subject: format!("sub-{}", plane.id.0),
    });
    built.capacity = Capacity::new(8_000, 32_768, 20).expect("capacity");
    built
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_workers_never_claim_the_same_plane() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let mut seeded = HashSet::new();
    for _ in 0..24 {
        seeded.insert(seed_customer_plane(&pool, &account).await.0);
    }

    let workers: Vec<_> = (0..4)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move {
                let mut taken = Vec::new();
                loop {
                    let batch = claim(&pool, 3600.0, 3).await;
                    if batch.is_empty() {
                        break;
                    }
                    taken.extend(batch);
                }
                taken
            })
        })
        .collect();

    let mut all = Vec::new();
    for worker in workers {
        all.extend(worker.await.expect("the worker finished"));
    }
    let distinct: HashSet<_> = all.iter().copied().collect();

    assert_eq!(all.len(), distinct.len(), "a plane was claimed twice");
    assert_eq!(distinct, seeded);
    assert!(claim(&pool, 3600.0, 10).await.is_empty());
    wipe(&pool).await;
}

#[tokio::test]
async fn a_stale_claim_is_taken_over_by_the_next_worker() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (plane, _) = seed_customer_plane(&pool, &account).await;

    assert_eq!(claim(&pool, 0.0, 10).await, vec![plane]);
    assert_eq!(claim(&pool, 3600.0, 10).await, vec![plane]);
    assert!(claim(&pool, 3600.0, 10).await.is_empty());
    wipe(&pool).await;
}

#[tokio::test]
async fn only_a_live_provisioning_customer_plane_is_claimed() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (bound, _) = seed_customer_plane(&pool, &account).await;
    let (failed, _) = seed_customer_plane(&pool, &account).await;
    let (removed, removed_deployment) = seed_customer_plane(&pool, &account).await;
    let (open, _) = seed_customer_plane(&pool, &account).await;

    with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        let claims = PostgresClusterClaims::new(&tx);
        assert!(
            claims
                .complete_provisioning(&built(&read_in(&tx, bound).await?))
                .await?
        );
        assert!(claims.fail_provisioning(&failed, "quota").await?);
        PostgresDeploymentRepository::new(&tx)
            .delete(removed_deployment)
            .await?;
        Ok(())
    })
    .await
    .expect("the setup ran");

    let claimed = claim(&pool, 3600.0, 10).await;

    assert_eq!(claimed, vec![open]);
    assert!(!claimed.contains(&removed));
    wipe(&pool).await;
}

async fn read_in(tx: &SharedTx<'_>, id: DataPlaneId) -> Result<DataPlane, CoreError> {
    PostgresDataPlaneRepository::new(tx)
        .find_by_id(&id)
        .await?
        .ok_or(CoreError::DataPlaneNotFound { id })
}

#[tokio::test]
async fn completing_applies_the_binding_and_capacity_and_stays_provisioning() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (id, _) = seed_customer_plane(&pool, &account).await;
    let target = built(&read(&pool, id).await);

    let applied = with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        PostgresClusterClaims::new(&tx)
            .complete_provisioning(&target)
            .await
    })
    .await
    .expect("the update ran");

    let stored = read(&pool, id).await;
    assert!(applied);
    assert_eq!(stored.status, DataPlaneStatus::Provisioning);
    assert_eq!(stored.herald, target.herald);
    assert_eq!(stored.capacity, target.capacity);
    assert_eq!(stored.last_seen_at, None);
    assert!(claim(&pool, 3600.0, 10).await.is_empty());
    wipe(&pool).await;
}

#[tokio::test]
async fn completing_never_overwrites_a_plane_that_failed_or_was_disabled() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (failed, _) = seed_customer_plane(&pool, &account).await;
    let (disabled, _) = seed_customer_plane(&pool, &account).await;

    let outcome = with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        let claims = PostgresClusterClaims::new(&tx);
        assert!(
            claims
                .fail_provisioning(&failed, "the node pool never became ready")
                .await?
        );
        let mut plane = read_in(&tx, disabled).await?;
        plane.disable();
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;

        let late_failed = claims
            .complete_provisioning(&built(&read_in(&tx, failed).await?))
            .await?;
        let late_disabled = claims
            .complete_provisioning(&built(&read_in(&tx, disabled).await?))
            .await?;
        let refail = claims.fail_provisioning(&disabled, "too late").await?;
        Ok((late_failed, late_disabled, refail))
    })
    .await
    .expect("the updates ran");

    assert_eq!(outcome, (false, false, false));
    let failed = read(&pool, failed).await;
    assert_eq!(failed.status, DataPlaneStatus::Failed);
    assert_eq!(
        failed.failure_reason.as_deref(),
        Some("the node pool never became ready")
    );
    assert_eq!(failed.herald, None);
    let disabled = read(&pool, disabled).await;
    assert_eq!(disabled.status, DataPlaneStatus::Disabled);
    assert_eq!(disabled.herald, None);
    wipe(&pool).await;
}

async fn record(pool: &PgPool, plane: DataPlaneId, provider_id: &str) -> ProvisionedResource {
    let resource = ProvisionedResource {
        kind: ResourceKind::Cluster,
        provider_id: provider_id.to_string(),
    };
    with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        PostgresClusterInventory::new(&tx)
            .record(&plane, &resource)
            .await
    })
    .await
    .expect("the resource was recorded");
    resource
}

#[tokio::test]
async fn teardown_lists_deleted_deployments_and_failed_planes_with_something_left() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (live, _) = seed_customer_plane(&pool, &account).await;
    let (deleted, deleted_deployment) = seed_customer_plane(&pool, &account).await;
    let (failed_built, _) = seed_customer_plane(&pool, &account).await;
    let (failed_empty, _) = seed_customer_plane(&pool, &account).await;
    record(&pool, live, "live-cluster").await;
    record(&pool, failed_built, "failed-cluster").await;

    with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        let claims = PostgresClusterClaims::new(&tx);
        claims.fail_provisioning(&failed_built, "quota").await?;
        claims.fail_provisioning(&failed_empty, "quota").await?;
        PostgresDeploymentRepository::new(&tx)
            .delete(deleted_deployment)
            .await
    })
    .await
    .expect("the setup ran");

    let listed: HashSet<_> = candidates(&pool).await.into_iter().collect();

    assert_eq!(listed, HashSet::from([deleted, failed_built]));
    assert!(!listed.contains(&live));
    assert!(!listed.contains(&failed_empty));
    wipe(&pool).await;
}

#[tokio::test]
async fn a_plane_being_provisioned_is_not_torn_down_and_a_disabled_one_is_left_alone() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (plane, deployment) = seed_customer_plane(&pool, &account).await;
    assert_eq!(claim(&pool, 3600.0, 10).await, vec![plane]);

    with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        PostgresDeploymentRepository::new(&tx)
            .delete(deployment)
            .await
    })
    .await
    .expect("the deployment was deleted");

    assert!(
        candidates(&pool).await.is_empty(),
        "a live claim is respected"
    );

    let disabled = with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        let claims = PostgresClusterClaims::new(&tx);
        claims.release_claim(&plane).await?;
        let first = claims.disable(&plane).await?;
        let second = claims.disable(&plane).await?;
        Ok((first, second))
    })
    .await
    .expect("the plane was disabled");

    assert_eq!(disabled, (true, false));
    assert_eq!(read(&pool, plane).await.status, DataPlaneStatus::Disabled);
    assert!(candidates(&pool).await.is_empty());
    wipe(&pool).await;
}

async fn awaiting(pool: &PgPool) -> Vec<DataPlaneId> {
    let listed = with_tx(pool, db_error, async |tx: SharedTx<'_>| {
        PostgresClusterClaims::new(&tx)
            .awaiting_deletion(1000)
            .await
    })
    .await
    .expect("the listing ran");
    mine(pool, listed).await
}

#[tokio::test]
async fn a_released_plane_of_a_deleting_deployment_awaits_its_deleted_status_until_it_is_set() {
    let _serial = COMMITTED.lock().await;
    let Some(pool) = pool().await else { return };
    let account = seed_account(&pool).await;
    let (live, _) = seed_customer_plane(&pool, &account).await;
    let (released, released_deployment) = seed_customer_plane(&pool, &account).await;
    let (holding, holding_deployment) = seed_customer_plane(&pool, &account).await;
    let (failed, failed_deployment) = seed_customer_plane(&pool, &account).await;
    record(&pool, holding, "still-there").await;

    with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        let claims = PostgresClusterClaims::new(&tx);
        let deployments = PostgresDeploymentRepository::new(&tx);
        deployments.delete(released_deployment).await?;
        deployments.delete(holding_deployment).await?;
        deployments.delete(failed_deployment).await?;
        claims.release_claim(&released).await?;
        claims.disable(&released).await?;
        claims.fail_provisioning(&failed, "quota").await?;
        Ok(())
    })
    .await
    .expect("the setup ran");

    let listed: HashSet<_> = awaiting(&pool).await.into_iter().collect();
    assert_eq!(listed, HashSet::from([released, failed]));
    assert!(!listed.contains(&live));
    assert!(!listed.contains(&holding));

    let confirmed = with_tx(&pool, db_error, async |tx: SharedTx<'_>| {
        let deployments = PostgresDeploymentRepository::new(&tx);
        let mut deployment = deployments
            .get_by_id(released_deployment)
            .await?
            .expect("the deployment exists");
        let first = deployment.confirm_deletion(Utc::now());
        deployments.update(deployment).await?;
        let again = deployments
            .get_by_id(released_deployment)
            .await?
            .expect("the deployment exists");
        Ok((first, again.status, again.deleted_at.is_some()))
    })
    .await
    .expect("the confirmation ran");

    assert_eq!(confirmed, (true, DeploymentStatus::Deleted, true));
    assert_eq!(awaiting(&pool).await, vec![failed]);
    wipe(&pool).await;
}
