use autharie_domain::{
    CoreError,
    organisation::OrganisationId,
    quotas::{
        Quota, QuotaKind,
        ports::{QuotaChange, QuotaRepository},
    },
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::quotas::PostgresQuotaRepository;
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

fn change() -> QuotaChange {
    QuotaChange {
        by: "operator-1".to_string(),
        at: Utc::now(),
    }
}

async fn an_organisation(tx: &SharedTx<'_>) -> Result<OrganisationId, CoreError> {
    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = Uuid::new_v4();

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'quotas', $3)")
        .bind(user_id)
        .bind(format!("{user_id}@quotas.test"))
        .bind(user_id.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'quotas', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
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
async fn an_organisation_without_an_override_has_none() {
    let pool = pool_or_skip!();

    let overrides = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        PostgresQuotaRepository::new(&tx)
            .overrides(organisation)
            .await
    })
    .await
    .expect("the transaction");

    assert!(overrides.is_empty());
}

#[tokio::test]
async fn an_override_is_read_back_as_a_limit_or_as_unlimited() {
    let pool = pool_or_skip!();

    let overrides = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let quotas = PostgresQuotaRepository::new(&tx);
        quotas
            .set_override(
                organisation,
                QuotaKind::McpServers,
                Some(Quota::Limited(3)),
                &change(),
            )
            .await?;
        quotas
            .set_override(
                organisation,
                QuotaKind::Policies,
                Some(Quota::Unlimited),
                &change(),
            )
            .await?;
        quotas
            .set_override(
                organisation,
                QuotaKind::Webhooks,
                Some(Quota::Limited(0)),
                &change(),
            )
            .await?;

        quotas.overrides(organisation).await
    })
    .await
    .expect("the transaction");

    assert_eq!(
        overrides.get(&QuotaKind::McpServers),
        Some(&Quota::Limited(3))
    );
    assert_eq!(overrides.get(&QuotaKind::Policies), Some(&Quota::Unlimited));
    assert_eq!(
        overrides.get(&QuotaKind::Webhooks),
        Some(&Quota::Limited(0)),
        "zero is a limit, not an absence"
    );
    assert_eq!(overrides.get(&QuotaKind::Users), None);
}

#[tokio::test]
async fn setting_an_override_twice_keeps_the_last_one() {
    let pool = pool_or_skip!();

    let overrides = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let quotas = PostgresQuotaRepository::new(&tx);
        for max in [3, 9] {
            quotas
                .set_override(
                    organisation,
                    QuotaKind::Policies,
                    Some(Quota::Limited(max)),
                    &change(),
                )
                .await?;
        }

        quotas.overrides(organisation).await
    })
    .await
    .expect("the transaction");

    assert_eq!(overrides.len(), 1);
    assert_eq!(
        overrides.get(&QuotaKind::Policies),
        Some(&Quota::Limited(9))
    );
}

#[tokio::test]
async fn clearing_an_override_returns_to_the_default() {
    let pool = pool_or_skip!();

    let overrides = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let quotas = PostgresQuotaRepository::new(&tx);
        quotas
            .set_override(
                organisation,
                QuotaKind::Policies,
                Some(Quota::Limited(9)),
                &change(),
            )
            .await?;
        quotas
            .set_override(organisation, QuotaKind::Policies, None, &change())
            .await?;

        quotas.overrides(organisation).await
    })
    .await
    .expect("the transaction");

    assert!(overrides.is_empty());
}

#[tokio::test]
async fn an_override_belongs_to_its_organisation() {
    let pool = pool_or_skip!();

    let other = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let another = an_organisation(&tx).await?;
        let quotas = PostgresQuotaRepository::new(&tx);
        quotas
            .set_override(
                organisation,
                QuotaKind::McpServers,
                Some(Quota::Limited(9)),
                &change(),
            )
            .await?;

        quotas.overrides(another).await
    })
    .await
    .expect("the transaction");

    assert!(other.is_empty());
}

#[tokio::test]
async fn an_override_is_removed_with_its_organisation() {
    let pool = pool_or_skip!();

    let left: Result<i64, CoreError> = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        PostgresQuotaRepository::new(&tx)
            .set_override(
                organisation,
                QuotaKind::Policies,
                Some(Quota::Limited(9)),
                &change(),
            )
            .await?;

        let mut guard = tx.lock().await;
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(organisation.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;

        sqlx::query_scalar("SELECT COUNT(*) FROM organisation_quotas WHERE organisation_id = $1")
            .bind(organisation.0)
            .fetch_one(&mut ***guard)
            .await
            .map_err(map_err)
    })
    .await;

    assert_eq!(left.expect("the transaction"), 0);
}
