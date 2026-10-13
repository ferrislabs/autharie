use autharie_domain::{
    CoreError,
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    cells::{AdministratorCredential, SealedAdministrator, Taken, TenantAdministratorRepository},
    dataplane::{
        credential_repository::SealedSecret,
        entities::DataPlane,
        ports::DataPlaneRepository,
        value_objects::{
            Capacity, DataPlaneAllocation, DataPlaneStatus, DeploymentResources, Region,
        },
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        environment::Environment, network::NetworkAccess, ports::DeploymentRepository,
    },
    organisation::OrganisationId,
    user::UserId,
    version::Version,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    cells::PostgresTenantAdministratorRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository,
};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

const FIRST: Uuid = Uuid::from_u128(1);
const SECOND: Uuid = Uuid::from_u128(2);

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

async fn a_deployment(tx: &SharedTx<'_>) -> Result<DeploymentId, CoreError> {
    let user = UserId(Uuid::new_v4());
    let organisation = OrganisationId(Uuid::new_v4());
    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'wa', $3)")
            .bind(user.0)
            .bind(format!("{}@wa.test", user.0))
            .bind(user.0.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(db_error)?;
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'wa', $2, $3, 'active', 'free', 100, 100, 100, now(), now())",
        )
        .bind(organisation.0)
        .bind(format!("wa-{}", organisation.0))
        .bind(user.0)
        .execute(&mut ***guard)
        .await
        .map_err(db_error)?;
    }

    let mut plane = DataPlane::new(
        DataPlaneAllocation::Shared,
        Region::new(format!("wa-{}", Uuid::new_v4())),
        Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
    );
    plane.status = DataPlaneStatus::Active;
    PostgresDataPlaneRepository::new(tx).save(&plane).await?;

    let at = Utc::now();
    let name = format!("wa{}", &Uuid::new_v4().simple().to_string()[..12]);
    let deployment = Deployment {
        topology: Default::default(),
        id: DeploymentId(Uuid::new_v4()),
        organisation_id: organisation,
        dataplane_id: plane.id,
        name: DeploymentName(name.clone()),
        kind: DeploymentKind::Ferriskey,
        version: Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: format!("wa-{name}"),
        environment: Environment::Development,
        offer: None,
        restored_from: None,
        resources: DeploymentResources::DEFAULT,
        created_by: user,
        created_at: at,
        updated_at: at,
        deployed_at: None,
        deleted_at: None,
        auto_upgrade: Default::default(),
        maintenance_window: None,
        network_access: NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    };
    let id = deployment.id;
    PostgresDeploymentRepository::new(tx)
        .insert(deployment)
        .await?;
    Ok(id)
}

fn sealed(
    ciphertext: &[u8],
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> SealedAdministrator {
    SealedAdministrator {
        username: "admin".to_string(),
        sealed: SealedSecret {
            ciphertext: ciphertext.to_vec(),
            nonce: vec![1; 12],
            wrapped_dek: WrappedDek::new("vault:v1:wrapped"),
            key: KeyRef::new(
                ProviderName::new("openbao"),
                KeyName::new("tenant-administrators").expect("a key name"),
                KeyVersion::new(1),
            ),
        },
        issued_at,
        expires_at,
    }
}

async fn material_left(tx: &SharedTx<'_>, deployment: DeploymentId) -> Result<bool, CoreError> {
    let mut guard = tx.lock().await;
    sqlx::query_scalar::<_, bool>(
        "SELECT ciphertext IS NOT NULL OR wrapped_dek IS NOT NULL \
         FROM tenant_administrators WHERE deployment_id = $1",
    )
    .bind(deployment.0)
    .fetch_one(&mut ***guard)
    .await
    .map_err(db_error)
}

async fn state(
    repository: &PostgresTenantAdministratorRepository<'_>,
    deployment: DeploymentId,
    now: DateTime<Utc>,
) -> Result<Option<(AdministratorCredential, bool)>, CoreError> {
    Ok(repository
        .describe(deployment, now)
        .await?
        .map(|administrator| (administrator.credential, administrator.stale)))
}

#[tokio::test]
async fn an_issued_credential_is_kept_only_when_asked_for_and_handed_over_once() {
    scratch(async |tx| {
        let deployment = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);
        let now = Utc::now();
        let later = now + Duration::days(7);

        assert!(
            !repository
                .store_issued(deployment, FIRST, &sealed(b"unasked", now, later))
                .await?
        );
        assert_eq!(state(&repository, deployment, now).await?, None);

        repository.request_issue(deployment, FIRST, now).await?;
        assert_eq!(
            state(&repository, deployment, now).await?,
            Some((AdministratorCredential::Issuing, false))
        );
        assert_eq!(repository.take(deployment, now).await?, Taken::Absent);

        assert!(
            repository
                .store_issued(deployment, FIRST, &sealed(b"first", now, later))
                .await?
        );
        assert!(
            !repository
                .store_issued(deployment, FIRST, &sealed(b"late", now, later))
                .await?
        );
        assert_eq!(
            state(&repository, deployment, now).await?,
            Some((AdministratorCredential::Ready, false))
        );

        let Taken::Sealed(taken) = repository.take(deployment, now).await? else {
            panic!("the credential was there to take");
        };
        assert_eq!(taken.sealed.ciphertext, b"first");
        assert_eq!(taken.username, "admin");
        assert!(!material_left(&tx, deployment).await?);

        assert_eq!(repository.take(deployment, now).await?, Taken::Revealed);
        assert_eq!(
            state(&repository, deployment, now).await?,
            Some((AdministratorCredential::Revealed, false))
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_new_issue_keeps_the_previous_credential_sealed_but_hides_it_until_the_new_one_arrives() {
    scratch(async |tx| {
        let deployment = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);
        let now = Utc::now();
        let later = now + Duration::days(7);

        repository.request_issue(deployment, FIRST, now).await?;
        repository
            .store_issued(deployment, FIRST, &sealed(b"first", now, later))
            .await?;
        repository.request_issue(deployment, SECOND, now).await?;

        assert!(material_left(&tx, deployment).await?);
        assert_eq!(
            state(&repository, deployment, now).await?,
            Some((AdministratorCredential::Issuing, false))
        );
        assert_eq!(repository.take(deployment, now).await?, Taken::Absent);
        assert!(material_left(&tx, deployment).await?);

        assert!(
            !repository
                .store_issued(deployment, FIRST, &sealed(b"late", now, later))
                .await?
        );
        assert!(
            repository
                .store_issued(deployment, SECOND, &sealed(b"second", now, later))
                .await?
        );
        let Taken::Sealed(taken) = repository.take(deployment, now).await? else {
            panic!("the newer credential was there to take");
        };
        assert_eq!(taken.sealed.ciphertext, b"second");
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn an_issue_that_never_arrives_turns_stale() {
    scratch(async |tx| {
        let deployment = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);
        let asked = Utc::now() - Duration::minutes(11);

        repository.request_issue(deployment, FIRST, asked).await?;

        assert_eq!(
            state(&repository, deployment, Utc::now()).await?,
            Some((AdministratorCredential::Issuing, true))
        );
        let record = repository
            .issue_record(deployment)
            .await?
            .expect("a record");
        assert!(record.may_issue_again(Utc::now()));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn an_expired_credential_is_never_handed_over_and_is_wiped() {
    scratch(async |tx| {
        let deployment = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);
        let now = Utc::now();

        repository
            .request_issue(deployment, FIRST, now - Duration::days(8))
            .await?;
        repository
            .store_issued(
                deployment,
                FIRST,
                &sealed(
                    b"stale",
                    now - Duration::days(8),
                    now - Duration::minutes(1),
                ),
            )
            .await?;

        assert_eq!(repository.take(deployment, now).await?, Taken::Expired);
        assert_eq!(
            state(&repository, deployment, now).await?,
            Some((AdministratorCredential::Expired, false))
        );
        assert!(!material_left(&tx, deployment).await?);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn the_purge_wipes_only_what_has_expired() {
    scratch(async |tx| {
        let expired = a_deployment(&tx).await?;
        let current = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);
        let now = Utc::now();

        for (deployment, expires_at) in [
            (expired, now - Duration::minutes(1)),
            (current, now + Duration::days(7)),
        ] {
            repository
                .request_issue(deployment, FIRST, now - Duration::days(8))
                .await?;
            repository
                .store_issued(
                    deployment,
                    FIRST,
                    &sealed(b"x", now - Duration::days(8), expires_at),
                )
                .await?;
        }

        assert!(repository.purge_expired(now).await? >= 1);
        assert!(!material_left(&tx, expired).await?);
        assert!(material_left(&tx, current).await?);
        assert_eq!(repository.purge_expired(now).await?, 0);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_deployment_without_an_administrator_has_nothing_to_take() {
    scratch(async |tx| {
        let deployment = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);

        assert_eq!(
            repository.take(deployment, Utc::now()).await?,
            Taken::Absent
        );
        assert_eq!(repository.describe(deployment, Utc::now()).await?, None);
        assert_eq!(repository.issue_record(deployment).await?, None);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_revealed_credential_cannot_hold_its_material_again() {
    scratch(async |tx| {
        let deployment = a_deployment(&tx).await?;
        let repository = PostgresTenantAdministratorRepository::new(&tx);
        let now = Utc::now();
        repository.request_issue(deployment, FIRST, now).await?;
        repository
            .store_issued(deployment, FIRST, &sealed(b"first", now, now + Duration::days(7)))
            .await?;
        repository.take(deployment, now).await?;

        let mut guard = tx.lock().await;
        let refused = sqlx::query(
            "UPDATE tenant_administrators SET ciphertext = 'x', nonce = '\\x000000000000000000000000', \
             wrapped_dek = 'w', key_provider = 'p', key_name = 'k', key_version = 1 \
             WHERE deployment_id = $1",
        )
        .bind(deployment.0)
        .execute(&mut ***guard)
        .await;
        assert!(refused.is_err());
        Ok(())
    })
    .await;
}
