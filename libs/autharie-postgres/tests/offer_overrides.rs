use autharie_domain::{
    CoreError,
    offers::{
        Offer,
        ports::{OfferChange, OfferOverrideRepository},
    },
    organisation::OrganisationId,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::offers::PostgresOfferOverrideRepository;
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

fn change() -> OfferChange {
    OfferChange {
        by: "operator-1".to_string(),
        at: Utc::now(),
    }
}

async fn an_organisation(tx: &SharedTx<'_>) -> Result<OrganisationId, CoreError> {
    let organisation_id = OrganisationId(Uuid::new_v4());
    let user_id = Uuid::new_v4();

    let mut guard = tx.lock().await;
    sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'offers', $3)")
        .bind(user_id)
        .bind(format!("{user_id}@offers.test"))
        .bind(user_id.to_string())
        .execute(&mut ***guard)
        .await
        .map_err(map_err)?;

    sqlx::query(
        "INSERT INTO organisations \
         (id, name, slug, owner_id, status, plan, max_instances, max_users, \
          max_storage_gb, created_at, updated_at) \
         VALUES ($1, 'offers', $2, $3, 'active', 'free', 5, 5, 5, now(), now())",
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
        PostgresOfferOverrideRepository::new(&tx)
            .overrides(organisation)
            .await
    })
    .await
    .expect("the transaction");

    assert!(overrides.is_empty());
}

#[tokio::test]
async fn an_override_opens_or_closes_and_is_read_back_either_way() {
    let pool = pool_or_skip!();

    let overrides = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let offers = PostgresOfferOverrideRepository::new(&tx);
        offers
            .set_override(organisation, Offer::Sandbox, Some(true), &change())
            .await?;
        offers
            .set_override(organisation, Offer::Standard, Some(false), &change())
            .await?;

        offers.overrides(organisation).await
    })
    .await
    .expect("the transaction");

    assert_eq!(overrides.get(&Offer::Sandbox), Some(&true));
    assert_eq!(overrides.get(&Offer::Standard), Some(&false));
    assert_eq!(overrides.get(&Offer::Scale), None);
}

#[tokio::test]
async fn setting_an_override_twice_keeps_the_last_one_and_clearing_removes_it() {
    let pool = pool_or_skip!();

    let (kept, cleared) = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let offers = PostgresOfferOverrideRepository::new(&tx);
        for open in [true, false] {
            offers
                .set_override(organisation, Offer::Sandbox, Some(open), &change())
                .await?;
        }
        let kept = offers.overrides(organisation).await?;
        offers
            .set_override(organisation, Offer::Sandbox, None, &change())
            .await?;

        Ok((kept, offers.overrides(organisation).await?))
    })
    .await
    .expect("the transaction");

    assert_eq!(kept.get(&Offer::Sandbox), Some(&false));
    assert!(cleared.is_empty());
}

#[tokio::test]
async fn an_override_belongs_to_its_organisation_and_goes_with_it() {
    let pool = pool_or_skip!();

    let (other, left): (_, i64) = in_scratch_tx(&pool, map_err, async |tx| {
        let organisation = an_organisation(&tx).await?;
        let another = an_organisation(&tx).await?;
        let offers = PostgresOfferOverrideRepository::new(&tx);
        offers
            .set_override(organisation, Offer::Sandbox, Some(true), &change())
            .await?;
        let other = offers.overrides(another).await?;

        let mut guard = tx.lock().await;
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(organisation.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        let left = sqlx::query_scalar(
            "SELECT COUNT(*) FROM organisation_offer_overrides WHERE organisation_id = $1",
        )
        .bind(organisation.0)
        .fetch_one(&mut ***guard)
        .await
        .map_err(map_err)?;

        Ok((other, left))
    })
    .await
    .expect("the transaction");

    assert!(other.is_empty());
    assert_eq!(left, 0);
}
