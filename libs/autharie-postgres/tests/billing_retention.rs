use autharie_domain::{
    CoreError,
    billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingName, BillingPeriod, Money, Statement, StatementId, StatementLine, StatementParts,
        StatementStatus, Subscription, SubscriptionTerms, TierCode,
        ports::{BillingAccountRepository, StatementRepository, SubscriptionRepository},
    },
    deployments::{DeploymentId, ports::DeploymentRepository},
    offers::Offer,
    organisation::OrganisationId,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::{
    billing::{
        PostgresBillingAccountRepository, PostgresStatementRepository,
        PostgresSubscriptionRepository,
    },
    deployments::PostgresDeploymentRepository,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
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

fn at(month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap()
}

struct Billed {
    organisation: OrganisationId,
    deployment: DeploymentId,
    subscription: Subscription,
    statement: Statement,
}

async fn billed_deleted_deployment(tx: &SharedTx<'_>) -> Result<Billed, CoreError> {
    let user_id = Uuid::new_v4();
    let organisation = OrganisationId(Uuid::new_v4());
    let dataplane_id = Uuid::new_v4();
    let deployment = DeploymentId(Uuid::new_v4());

    {
        let mut guard = tx.lock().await;
        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'retention', $3)")
            .bind(user_id)
            .bind(format!("{user_id}@retention.test"))
            .bind(user_id.to_string())
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'retention', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
        )
        .bind(organisation.0)
        .bind(organisation.0.to_string())
        .bind(user_id)
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;
        sqlx::query(
            "INSERT INTO data_planes \
             (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
              capacity_storage_gib) \
             VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200)",
        )
        .bind(dataplane_id)
        .bind(format!("retention-{dataplane_id}"))
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;
        sqlx::query(
            "INSERT INTO deployments \
             (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
              created_by, cpu_millis, memory_mib, storage_gib, hostname_slug, deleted_at, \
              updated_at) \
             VALUES ($1, $2, $3, 'retention', 'ferriskey', 'deleted', 'retention-test', \
                     '26.0.1', $4, 250, 512, 1, $5, now() - interval '90 days', \
                     now() - interval '90 days')",
        )
        .bind(deployment.0)
        .bind(organisation.0)
        .bind(dataplane_id)
        .bind(user_id)
        .bind(format!("retention-{deployment}", deployment = deployment.0))
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;
    }

    let subscription = Subscription::open(
        organisation,
        deployment,
        SubscriptionTerms {
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(4900),
            offer: Offer::Standard,
            features: Default::default(),
        },
        at(10, 1),
    );
    PostgresSubscriptionRepository::new(tx)
        .open(&subscription)
        .await?;

    let period = BillingPeriod::new(2026, 10).unwrap();
    let statement = Statement::from(StatementParts {
        id: StatementId::new(),
        organisation_id: organisation,
        period,
        lines: vec![StatementLine {
            deployment_id: deployment,
            subscription_id: subscription.id(),
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(4900),
            starts_at: period.starts_at(),
            ends_at: period.ends_at(),
            covered_seconds: period.seconds(),
            amount: Money::from_minor(4900),
        }],
        status: StatementStatus::Held,
        invoice: None,
        created_at: at(11, 1),
        updated_at: at(11, 1),
    });
    PostgresStatementRepository::new(tx)
        .insert(&statement)
        .await?;

    PostgresBillingAccountRepository::new(tx)
        .save(&BillingAccount::new(
            organisation,
            BillingDetails {
                name: BillingName::parse("Retention SAS").unwrap(),
                email: BillingEmail::parse("billing@retention.test").unwrap(),
                address: BillingAddress::parse(BillingAddressInput {
                    line1: "1 rue de la Paix",
                    line2: None,
                    postal_code: "75002",
                    city: "Paris",
                    country: "FR",
                })
                .unwrap(),
                vat_number: None,
                siren: None,
            },
        ))
        .await?;

    Ok(Billed {
        organisation,
        deployment,
        subscription,
        statement,
    })
}

async fn count(tx: &SharedTx<'_>, table: &str, column: &str, id: Uuid) -> Result<i64, CoreError> {
    let mut guard = tx.lock().await;
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE {column} = $1"))
        .bind(id)
        .fetch_one(&mut ***guard)
        .await
        .map_err(map_err)
}

struct Remaining {
    deployments: i64,
    subscription: Option<Subscription>,
    statement: Option<Statement>,
    lines: i64,
    accounts: i64,
}

async fn what_remains(tx: &SharedTx<'_>, billed: &Billed) -> Result<Remaining, CoreError> {
    Ok(Remaining {
        deployments: count(tx, "deployments", "id", billed.deployment.0).await?,
        subscription: PostgresSubscriptionRepository::new(tx)
            .latest_for(&billed.deployment)
            .await?,
        statement: PostgresStatementRepository::new(tx)
            .find(&billed.statement.id())
            .await?,
        lines: count(
            tx,
            "billing_statement_lines",
            "statement_id",
            billed.statement.id().0,
        )
        .await?,
        accounts: count(
            tx,
            "billing_accounts",
            "organisation_id",
            billed.organisation.0,
        )
        .await?,
    })
}

#[tokio::test]
async fn purging_a_deleted_deployment_leaves_its_subscriptions_and_statement_lines() {
    let pool = pool_or_skip!();

    let (purged, remaining, billed) = in_scratch_tx(&pool, map_err, async |tx| {
        let billed = billed_deleted_deployment(&tx).await?;

        let purged = PostgresDeploymentRepository::new(&tx)
            .purge_deleted(Utc::now() - Duration::days(30))
            .await?;

        Ok((purged, what_remains(&tx, &billed).await?, billed))
    })
    .await
    .expect("the scratch transaction ran");

    assert!(purged >= 1, "the fixture is past its retention");
    assert_eq!(remaining.deployments, 0);
    assert_eq!(remaining.subscription, Some(billed.subscription));
    assert_eq!(remaining.statement, Some(billed.statement));
    assert_eq!(remaining.lines, 1);
    assert_eq!(remaining.accounts, 1);
}

#[tokio::test]
async fn deleting_a_deployment_row_leaves_its_subscriptions_and_statement_lines() {
    let pool = pool_or_skip!();

    let (remaining, billed) = in_scratch_tx(&pool, map_err, async |tx| {
        let billed = billed_deleted_deployment(&tx).await?;

        {
            let mut guard = tx.lock().await;
            sqlx::query("DELETE FROM deployments WHERE id = $1")
                .bind(billed.deployment.0)
                .execute(&mut ***guard)
                .await
                .map_err(map_err)?;
        }

        Ok((what_remains(&tx, &billed).await?, billed))
    })
    .await
    .expect("the scratch transaction ran");

    assert_eq!(remaining.deployments, 0);
    assert_eq!(remaining.subscription, Some(billed.subscription));
    assert_eq!(remaining.statement, Some(billed.statement));
    assert_eq!(remaining.lines, 1);
}

#[tokio::test]
async fn deleting_an_organisation_leaves_its_billing_rows() {
    let pool = pool_or_skip!();

    let (remaining, billed) = in_scratch_tx(&pool, map_err, async |tx| {
        let billed = billed_deleted_deployment(&tx).await?;

        {
            let mut guard = tx.lock().await;
            sqlx::query("DELETE FROM organisations WHERE id = $1")
                .bind(billed.organisation.0)
                .execute(&mut ***guard)
                .await
                .map_err(map_err)?;
        }

        Ok((what_remains(&tx, &billed).await?, billed))
    })
    .await
    .expect("the scratch transaction ran");

    assert_eq!(
        remaining.deployments, 0,
        "the deployment went with its organisation"
    );
    assert_eq!(remaining.subscription, Some(billed.subscription));
    assert_eq!(remaining.statement, Some(billed.statement));
    assert_eq!(remaining.lines, 1);
    assert_eq!(remaining.accounts, 1);
}
