use std::time::Duration;

use autharie_domain::{
    CoreError,
    agents::{
        server_ports::AgentServerRepository,
        servers::{CustomerServer, CustomerServerUrl, ServerId, ServerSlug},
    },
    dataplane::{
        ports::DataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneStatus, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::DeploymentRepository,
    },
    offers::Offer,
    organisation::OrganisationId,
    quotas::{
        Quota, QuotaKind,
        ports::{QuotaChange, QuotaRepository},
        service::require_room,
    },
    user::UserId,
};
use autharie_persistence::with_tx;
use autharie_postgres::{
    agents::PostgresAgentRepository, dataplane::PostgresDataPlaneRepository,
    deployments::PostgresDeploymentRepository, quotas::PostgresQuotaRepository,
};
use chrono::Utc;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const REGION: &str = "creates-serialised-test";
const MAX: u32 = 3;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());

    let Some(url) = url else {
        assert!(
            std::env::var("REQUIRE_DATABASE_URL").is_err(),
            "REQUIRE_DATABASE_URL is set but DATABASE_URL is not: these tests \
             would have skipped and reported success"
        );
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };

    Some(
        PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but the database is unreachable"),
    )
}

fn map_err(e: sqlx::Error) -> CoreError {
    CoreError::DatabaseError {
        message: e.to_string(),
    }
}

fn server(deployment_id: DeploymentId, slug: &str) -> CustomerServer {
    let at = Utc::now();
    CustomerServer {
        id: ServerId(Uuid::new_v4()),
        deployment_id,
        slug: ServerSlug::try_from(slug).unwrap(),
        name: slug.to_uppercase(),
        url: CustomerServerUrl::try_from("https://tools.example.com/mcp").unwrap(),
        enabled: true,
        mcp_url: None,
        discovery: None,
        discovery_requested_at: Some(at),
        created_at: at,
        updated_at: at,
    }
}

struct Seeded {
    organisation: OrganisationId,
    user: UserId,
    dataplane: DataPlaneId,
    deployment: DeploymentId,
}

async fn seed(pool: &PgPool) -> Seeded {
    let user = UserId(Uuid::new_v4());
    let organisation = OrganisationId(Uuid::new_v4());

    with_tx(pool, map_err, async |tx| {
        {
            let mut guard = tx.lock().await;
            sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'creates', $3)")
                .bind(user.0)
                .bind(format!("{}@creates.test", user.0))
                .bind(user.0.to_string())
                .execute(&mut ***guard)
                .await
                .map_err(map_err)?;
            sqlx::query(
                "INSERT INTO organisations \
                 (id, name, slug, owner_id, status, plan, max_instances, max_users, \
                  max_storage_gb, created_at, updated_at) \
                 VALUES ($1, 'creates', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
            )
            .bind(organisation.0)
            .bind(organisation.0.to_string())
            .bind(user.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        let mut plane = autharie_domain::dataplane::entities::DataPlane::new(
            DataPlaneAllocation::Shared,
            Region::new(REGION),
            Capacity::new(64_000, 131_072, 2_000).expect("non-zero capacity"),
        );
        plane.status = DataPlaneStatus::Active;
        PostgresDataPlaneRepository::new(&tx).save(&plane).await?;

        let at = Utc::now();
        let deployment = Deployment {
            topology: Default::default(),
            id: DeploymentId(Uuid::new_v4()),
            organisation_id: organisation,
            dataplane_id: plane.id,
            name: DeploymentName("serialised".to_string()),
            kind: DeploymentKind::Ferriskey,
            version: autharie_domain::version::Version::new(26, 0, 1),
            status: DeploymentStatus::Successful,
            namespace: "creates-serialised".to_string(),
            environment: autharie_domain::deployments::environment::Environment::Development,
            offer: Some(Offer::Free),
            restored_from: None,
            failure_reason: None,
            resources: autharie_domain::dataplane::value_objects::DeploymentResources::DEFAULT,
            created_by: user,
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
            distribution: Default::default(),
        };
        PostgresDeploymentRepository::new(&tx)
            .insert(deployment.clone())
            .await?;

        let quotas = PostgresQuotaRepository::new(&tx);
        quotas
            .set_override(
                organisation,
                QuotaKind::McpServers,
                Some(Quota::Limited(MAX)),
                &QuotaChange {
                    by: "test".to_string(),
                    at,
                },
            )
            .await?;
        let agents = PostgresAgentRepository::new(&tx);
        for slug in (0..MAX - 1).map(|n| format!("seeded-{n}")) {
            agents.insert_server(&server(deployment.id, &slug)).await?;
        }

        Ok(Seeded {
            organisation,
            user,
            dataplane: plane.id,
            deployment: deployment.id,
        })
    })
    .await
    .expect("the seed is committed")
}

async fn forget(pool: &PgPool, seeded: &Seeded) {
    sqlx::query("DELETE FROM deployments WHERE organisation_id = $1")
        .bind(seeded.organisation.0)
        .execute(pool)
        .await
        .expect("the deployment goes");
    sqlx::query("DELETE FROM data_planes WHERE id = $1")
        .bind(seeded.dataplane.0)
        .execute(pool)
        .await
        .expect("the data plane goes");
    sqlx::query("DELETE FROM organisations WHERE id = $1")
        .bind(seeded.organisation.0)
        .execute(pool)
        .await
        .expect("the organisation goes");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(seeded.user.0)
        .execute(pool)
        .await
        .expect("the user goes");
}

async fn create_one(pool: PgPool, seeded: &Seeded, slug: String) -> Result<(), CoreError> {
    let deployment = seeded.deployment;
    let organisation = seeded.organisation;

    with_tx(&pool, map_err, async |tx| {
        let quotas = PostgresQuotaRepository::new(&tx);
        let agents = PostgresAgentRepository::new(&tx);

        quotas
            .serialise_creates(deployment.0, QuotaKind::McpServers.as_str())
            .await?;
        let held = agents.list_servers(deployment).await?.len();
        require_room(
            &quotas,
            organisation,
            Some(Offer::Free),
            QuotaKind::McpServers,
            held,
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(300)).await;

        agents.insert_server(&server(deployment, &slug)).await
    })
    .await
}

#[tokio::test]
async fn two_concurrent_creates_at_max_minus_one_leave_exactly_max() {
    let Some(pool) = pool().await else {
        return;
    };
    let seeded = seed(&pool).await;

    let (first, second) = tokio::join!(
        create_one(pool.clone(), &seeded, "racing-a".to_string()),
        create_one(pool.clone(), &seeded, "racing-b".to_string()),
    );

    let stored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_servers WHERE deployment_id = $1")
            .bind(seeded.deployment.0)
            .fetch_one(&pool)
            .await
            .expect("the count");
    forget(&pool, &seeded).await;

    let refusals = [&first, &second]
        .into_iter()
        .filter(|result| matches!(result, Err(CoreError::OrganisationLimitReached { .. })))
        .count();
    assert_eq!(
        (first.is_ok() as usize + second.is_ok() as usize, refusals),
        (1, 1),
        "{first:?} {second:?}"
    );
    assert_eq!(stored, i64::from(MAX));
}
