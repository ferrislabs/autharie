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
            CloudCredentialRepository, DataPlaneFailures, SealedCredential, SealedSecret,
            SealedSecretRepository,
        },
        entities::DataPlane,
        inventory::{ClusterInventory, ProvisionedResource, ResourceKind},
        ports::DataPlaneRepository,
        value_objects::{
            Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneMode, DataPlaneStatus,
            DeploymentResources, PlacementPolicy, PlacementRequest, Region,
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
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    credentials::{PostgresCloudCredentialRepository, PostgresSealedSecretRepository},
    dataplane::{PostgresClusterInventory, PostgresDataPlaneRepository},
    deployments::PostgresDeploymentRepository,
};
use chrono::{Duration, Utc};
use uuid::Uuid;

mod support;
use support::pool;

const REGION: &str = "customer-cloud-test";

fn db_error(error: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: error.to_string(),
    }
}

async fn seed_organisation(tx: &SharedTx<'_>) -> Result<(UserId, OrganisationId), CoreError> {
    let user_id = UserId(Uuid::new_v4());
    let organisation_id = OrganisationId(Uuid::new_v4());

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'cloud', $3)")
        .bind(user_id.0)
        .bind(format!("{}@customer-cloud.test", user_id.0))
        .bind(user_id.0.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(db_error)?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'cloud', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
    )
    .bind(organisation_id.0)
    .bind(format!("cloud-{}", organisation_id.0))
    .bind(user_id.0)
    .execute(&mut ***guard)
    .await
    .map_err(db_error)?;

    Ok((user_id, organisation_id))
}

async fn seed_credential(
    tx: &SharedTx<'_>,
    organisation_id: OrganisationId,
) -> Result<CloudCredentialId, CoreError> {
    let id = CloudCredentialId(Uuid::new_v4());

    PostgresSealedSecretRepository::new(tx)
        .insert(&SealedCredential {
            id,
            organisation_id,
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

    PostgresCloudCredentialRepository::new(tx)
        .insert(&CloudCredential {
            id,
            organisation_id,
            provider: Provider::Scaleway,
            label: "production".to_string(),
            scope_check: ScopeCheck {
                checked_at: Utc::now(),
            },
            created_at: Utc::now(),
        })
        .await?;

    Ok(id)
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

fn customer_plane(
    organisation_id: OrganisationId,
    deployment_id: DeploymentId,
    credential_id: CloudCredentialId,
) -> DataPlane {
    DataPlane::new(
        DataPlaneAllocation::Customer {
            organisation_id,
            deployment_id,
            credential_id,
        },
        Region::new(REGION),
        Capacity::new(8_000, 16_384, 200).expect("non-zero capacity"),
    )
}

fn deployment(
    id: DeploymentId,
    organisation_id: OrganisationId,
    dataplane_id: DataPlaneId,
    created_by: UserId,
    distribution: Distribution,
) -> Deployment {
    let at = Utc::now();

    Deployment {
        id,
        organisation_id,
        dataplane_id,
        name: DeploymentName(format!("cloud-{}", id.0)),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: format!("cloud-{}", id.0),
        environment: autharie_domain::deployments::environment::Environment::Development,
        offer: None,
        restored_from: None,
        resources: DeploymentResources::DEFAULT,
        created_by,
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
        distribution,
    }
}

async fn shared_plane(tx: &SharedTx<'_>) -> Result<DataPlane, CoreError> {
    let mut plane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(REGION),
        Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
    );
    plane.status = DataPlaneStatus::Active;
    let repository = PostgresDataPlaneRepository::new(tx);
    repository.save(&plane).await?;
    repository
        .touch_last_seen(&plane.id, Utc::now(), None, None)
        .await?;
    Ok(plane)
}

async fn run<T>(
    body: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>,
) -> Option<Result<T, CoreError>> {
    let pool = pool().await?;
    Some(in_scratch_tx(&pool, db_error, body).await)
}

#[tokio::test]
async fn a_customer_cloud_deployment_round_trips_its_distribution() {
    let Some(result) = run(async |tx| {
        let (user, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let deployment_id = DeploymentId(Uuid::new_v4());
        let plane = customer_plane(organisation, deployment_id, credential);
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;

        let distribution = Distribution::CustomerCloud {
            credential_id: credential,
            profile: profile(),
        };
        let repository = PostgresDeploymentRepository::new(&tx);
        repository
            .insert(deployment(
                deployment_id,
                organisation,
                plane.id,
                user,
                distribution.clone(),
            ))
            .await?;

        let read = repository
            .get_by_id(deployment_id)
            .await?
            .expect("it was inserted");
        let listed = repository.list_by_organisation(organisation).await?;

        Ok((distribution, read.distribution, listed))
    })
    .await
    else {
        return;
    };

    let (expected, read, listed) = result.expect("the transaction ran");
    assert_eq!(read, expected);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].distribution, expected);
}

#[tokio::test]
async fn shared_and_self_hosted_deployments_round_trip_their_distribution() {
    let Some(result) = run(async |tx| {
        let (user, organisation) = seed_organisation(&tx).await?;
        let plane = shared_plane(&tx).await?;
        let repository = PostgresDeploymentRepository::new(&tx);

        let mut read = Vec::new();
        for distribution in [Distribution::Shared, Distribution::SelfHosted] {
            let id = DeploymentId(Uuid::new_v4());
            repository
                .insert(deployment(id, organisation, plane.id, user, distribution))
                .await?;
            read.push(
                repository
                    .get_by_id(id)
                    .await?
                    .expect("inserted")
                    .distribution,
            );
        }

        Ok(read)
    })
    .await
    else {
        return;
    };

    assert_eq!(
        result.expect("the transaction ran"),
        vec![Distribution::Shared, Distribution::SelfHosted]
    );
}

#[tokio::test]
async fn updating_a_deployment_keeps_its_distribution() {
    let Some(result) = run(async |tx| {
        let (user, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let deployment_id = DeploymentId(Uuid::new_v4());
        let plane = customer_plane(organisation, deployment_id, credential);
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;

        let repository = PostgresDeploymentRepository::new(&tx);
        let mut stored = deployment(
            deployment_id,
            organisation,
            plane.id,
            user,
            Distribution::CustomerCloud {
                credential_id: credential,
                profile: profile(),
            },
        );
        repository.insert(stored.clone()).await?;

        stored.status = DeploymentStatus::Maintenance;
        repository.update(stored.clone()).await?;

        let read = repository.get_by_id(deployment_id).await?.expect("stored");
        Ok((stored.distribution, read))
    })
    .await
    else {
        return;
    };

    let (expected, read) = result.expect("the transaction ran");
    assert_eq!(read.distribution, expected);
    assert_eq!(read.status, DeploymentStatus::Maintenance);
}

#[tokio::test]
async fn a_customer_data_plane_round_trips_its_allocation() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let deployment_id = DeploymentId(Uuid::new_v4());
        let plane = customer_plane(organisation, deployment_id, credential);
        let repository = PostgresDataPlaneRepository::new(&tx);
        repository.save(&plane).await?;

        let read = repository.find_by_id(&plane.id).await?.expect("saved");
        Ok((plane.allocation, read))
    })
    .await
    else {
        return;
    };

    let (expected, read) = result.expect("the transaction ran");
    assert_eq!(read.allocation, expected);
    assert_eq!(read.status, DataPlaneStatus::Provisioning);
    assert_eq!(read.failure_reason, None);
}

#[tokio::test]
async fn a_failed_data_plane_keeps_a_readable_reason_spec_ccp_11() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let plane = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        let repository = PostgresDataPlaneRepository::new(&tx);
        repository.save(&plane).await?;

        let marked = repository
            .mark_failed(
                &plane.id,
                "the provider quota in this account does not allow this cluster",
            )
            .await?;
        let missing = repository
            .mark_failed(&DataPlaneId(Uuid::new_v4()), "nothing there")
            .await?;
        let read = repository.find_by_id(&plane.id).await?.expect("saved");
        let listed = repository.list_all().await?;

        let mut failed = plane.clone();
        failed.fail("saved through save");
        repository.save(&failed).await?;
        let resaved = repository.find_by_id(&plane.id).await?.expect("saved");

        Ok((marked, missing, read, listed, resaved))
    })
    .await
    else {
        return;
    };

    let (marked, missing, read, listed, resaved) = result.expect("the transaction ran");
    assert!(marked);
    assert!(!missing);
    assert_eq!(read.status, DataPlaneStatus::Failed);
    assert_eq!(
        read.failure_reason.as_deref(),
        Some("the provider quota in this account does not allow this cluster")
    );
    assert!(
        listed
            .iter()
            .any(|plane| plane.id == read.id && plane.failure_reason == read.failure_reason)
    );
    assert_eq!(
        resaved.failure_reason.as_deref(),
        Some("saved through save")
    );
}

#[tokio::test]
async fn a_reason_without_a_failure_is_refused_by_the_database() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let mut plane = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        plane.failure_reason = Some("but still provisioning".to_string());

        Ok(PostgresDataPlaneRepository::new(&tx)
            .save(&plane)
            .await
            .is_err())
    })
    .await
    else {
        return;
    };

    assert!(result.expect("the transaction ran"));
}

#[tokio::test]
async fn placement_never_returns_a_customer_data_plane() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let mut plane = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        plane.status = DataPlaneStatus::Active;
        let repository = PostgresDataPlaneRepository::new(&tx);
        repository.save(&plane).await?;
        repository
            .touch_last_seen(&plane.id, Utc::now(), None, None)
            .await?;

        let mut placed = Vec::new();
        for mode in [DataPlaneMode::Shared, DataPlaneMode::Dedicated] {
            for region in [Some(Region::new(REGION)), None] {
                placed.push(
                    repository
                        .find_available(PlacementRequest {
                            region,
                            organisation_id: organisation,
                            mode,
                            resources: DeploymentResources::DEFAULT,
                            policy: PlacementPolicy::Spread,
                            seen_since: Utc::now() - Duration::minutes(5),
                        })
                        .await?
                        .map(|found| found.id),
                );
            }
        }

        let shared = repository
            .find_active_shared_by_region(&Region::new(REGION))
            .await?;
        let dedicated = repository
            .find_dedicated_for_organisation(&organisation, &Region::new(REGION))
            .await?;
        let served = repository.region_is_served(&Region::new(REGION)).await?;
        let blocked = repository
            .region_blocked_by_deployment_count(
                &Region::new(REGION),
                DataPlaneMode::Dedicated,
                DeploymentResources::DEFAULT,
            )
            .await?;

        Ok((placed, shared, dedicated, served, blocked))
    })
    .await
    else {
        return;
    };

    let (placed, shared, dedicated, served, blocked) = result.expect("the transaction ran");
    assert!(placed.iter().all(Option::is_none), "{placed:?}");
    assert!(shared.is_empty());
    assert!(dedicated.is_none());
    assert!(!served);
    assert!(!blocked);
}

#[tokio::test]
async fn a_shared_data_plane_in_the_same_region_is_still_placed() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let mut customer = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        customer.status = DataPlaneStatus::Active;
        let repository = PostgresDataPlaneRepository::new(&tx);
        repository.save(&customer).await?;
        repository
            .touch_last_seen(&customer.id, Utc::now(), None, None)
            .await?;
        let shared = shared_plane(&tx).await?;

        let found = repository
            .find_available(PlacementRequest {
                region: Some(Region::new(REGION)),
                organisation_id: organisation,
                mode: DataPlaneMode::Shared,
                resources: DeploymentResources::DEFAULT,
                policy: PlacementPolicy::Spread,
                seen_since: Utc::now() - Duration::minutes(5),
            })
            .await?
            .map(|found| found.id);

        Ok((shared.id, found))
    })
    .await
    else {
        return;
    };

    let (shared, found) = result.expect("the transaction ran");
    assert_eq!(found, Some(shared));
}

fn resource(kind: ResourceKind, id: &str) -> ProvisionedResource {
    ProvisionedResource {
        kind,
        provider_id: id.to_string(),
    }
}

#[tokio::test]
async fn the_inventory_records_each_resource_once_and_releases_it() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let plane = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;
        let inventory = PostgresClusterInventory::new(&tx);

        let network = resource(ResourceKind::PrivateNetwork, "pn-1");
        let cluster = resource(ResourceKind::Cluster, "cl-1");
        let pool = resource(ResourceKind::NodePool, "np-1");
        inventory.record(&plane.id, &network).await?;
        inventory.record(&plane.id, &cluster).await?;
        inventory.record(&plane.id, &cluster).await?;
        inventory.record(&plane.id, &pool).await?;

        let before = inventory.unreleased(&plane.id).await?;

        inventory.mark_released(&plane.id, &pool).await?;
        inventory.mark_released(&plane.id, &pool).await?;
        inventory
            .mark_released(
                &plane.id,
                &resource(ResourceKind::Cluster, "never-recorded"),
            )
            .await?;
        let after = inventory.unreleased(&plane.id).await?;

        inventory.mark_released(&plane.id, &cluster).await?;
        inventory.mark_released(&plane.id, &network).await?;
        let drained = inventory.unreleased(&plane.id).await?;

        let elsewhere = inventory.unreleased(&DataPlaneId(Uuid::new_v4())).await?;

        Ok((before, after, drained, elsewhere))
    })
    .await
    else {
        return;
    };

    let (before, after, drained, elsewhere) = result.expect("the transaction ran");
    assert_eq!(before.len(), 3, "a resource recorded twice is one resource");
    assert_eq!(after.len(), 2);
    assert!(!after.contains(&resource(ResourceKind::NodePool, "np-1")));
    assert!(drained.is_empty(), "nothing is left after every release");
    assert!(elsewhere.is_empty());
}

#[tokio::test]
async fn the_same_provider_id_of_another_kind_is_another_resource() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let plane = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;
        let inventory = PostgresClusterInventory::new(&tx);

        inventory
            .record(&plane.id, &resource(ResourceKind::Cluster, "same"))
            .await?;
        inventory
            .record(&plane.id, &resource(ResourceKind::NodePool, "same"))
            .await?;

        inventory.unreleased(&plane.id).await
    })
    .await
    else {
        return;
    };

    assert_eq!(result.expect("the transaction ran").len(), 2);
}

#[tokio::test]
async fn a_credential_is_listed_for_its_organisation_only() {
    let Some(result) = run(async |tx| {
        let (_, mine) = seed_organisation(&tx).await?;
        let (_, theirs) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, mine).await?;
        seed_credential(&tx, theirs).await?;
        let repository = PostgresCloudCredentialRepository::new(&tx);

        let listed = repository.list_for_organisation(&mine).await?;
        let read = repository.get(&credential).await?;
        let absent = repository.get(&CloudCredentialId(Uuid::new_v4())).await?;

        Ok((credential, listed, read, absent))
    })
    .await
    else {
        return;
    };

    let (credential, listed, read, absent) = result.expect("the transaction ran");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, credential);
    assert_eq!(listed[0].label, "production");
    assert_eq!(read.map(|found| found.id), Some(credential));
    assert!(absent.is_none());
}

#[tokio::test]
async fn a_sealed_secret_comes_back_as_it_went_in() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let repository = PostgresSealedSecretRepository::new(&tx);

        let read = repository.get(&credential).await?;
        let deleted = repository.delete(&credential).await?;
        let deleted_again = repository.delete(&credential).await?;
        let after = repository.get(&credential).await?;
        let metadata = PostgresCloudCredentialRepository::new(&tx)
            .get(&credential)
            .await?;

        Ok((read, deleted, deleted_again, after, metadata))
    })
    .await
    else {
        return;
    };

    let (read, deleted, deleted_again, after, metadata) = result.expect("the transaction ran");
    let read = read.expect("stored");
    assert_eq!(read.sealed.ciphertext, vec![1, 2, 3, 4]);
    assert_eq!(read.sealed.nonce, vec![9; 12]);
    assert_eq!(read.sealed.wrapped_dek.as_str(), "vault:v1:opaque");
    assert_eq!(read.sealed.key.version, KeyVersion::new(1));
    assert!(deleted);
    assert!(!deleted_again);
    assert!(after.is_none());
    assert!(metadata.is_none(), "the metadata goes with the secret");
}

#[tokio::test]
async fn a_credential_used_by_a_live_deployment_is_in_use_spec_ccp_4() {
    let Some(result) = run(async |tx| {
        let (user, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let credentials = PostgresCloudCredentialRepository::new(&tx);
        let unused = credentials.is_in_use(&credential).await?;

        let deployment_id = DeploymentId(Uuid::new_v4());
        let plane = customer_plane(organisation, deployment_id, credential);
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;
        let deployments = PostgresDeploymentRepository::new(&tx);
        let mut stored = deployment(
            deployment_id,
            organisation,
            plane.id,
            user,
            Distribution::CustomerCloud {
                credential_id: credential,
                profile: profile(),
            },
        );
        deployments.insert(stored.clone()).await?;
        let used = credentials.is_in_use(&credential).await?;

        stored.status = DeploymentStatus::Deleting;
        deployments.update(stored.clone()).await?;
        let deleting = credentials.is_in_use(&credential).await?;

        stored.status = DeploymentStatus::Deleted;
        deployments.update(stored).await?;
        let deleted = credentials.is_in_use(&credential).await?;

        Ok((unused, used, deleting, deleted))
    })
    .await
    else {
        return;
    };

    let (unused, used, deleting, deleted) = result.expect("the transaction ran");
    assert!(!unused);
    assert!(used);
    assert!(
        deleting,
        "a cluster being torn down still needs its credential"
    );
    assert!(!deleted);
}

#[tokio::test]
async fn a_credential_with_resources_left_is_in_use_until_they_are_released() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let credential = seed_credential(&tx, organisation).await?;
        let plane = customer_plane(organisation, DeploymentId(Uuid::new_v4()), credential);
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;
        let inventory = PostgresClusterInventory::new(&tx);
        let credentials = PostgresCloudCredentialRepository::new(&tx);

        let cluster = resource(ResourceKind::Cluster, "cl-1");
        inventory.record(&plane.id, &cluster).await?;
        let holding = credentials.is_in_use(&credential).await?;

        inventory.mark_released(&plane.id, &cluster).await?;
        let released = credentials.is_in_use(&credential).await?;

        Ok((holding, released))
    })
    .await
    else {
        return;
    };

    let (holding, released) = result.expect("the transaction ran");
    assert!(holding);
    assert!(!released);
}

#[tokio::test]
async fn a_customer_data_plane_must_name_its_deployment_and_credential() {
    let Some(result) = run(async |tx| {
        let (_, organisation) = seed_organisation(&tx).await?;
        let mut guard = tx.lock().await;
        let refused = sqlx::query(
            "INSERT INTO data_planes (id, mode, region, status, organisation_id, \
             capacity_cpu_millis, capacity_memory_mib, capacity_storage_gib) \
             VALUES ($1, 'customer', $2, 'provisioning', $3, 1, 1, 1)",
        )
        .bind(Uuid::new_v4())
        .bind(REGION)
        .bind(organisation.0)
        .execute(&mut ***guard)
        .await
        .is_err();

        Ok(refused)
    })
    .await
    else {
        return;
    };

    assert!(result.expect("the transaction ran"));
}
