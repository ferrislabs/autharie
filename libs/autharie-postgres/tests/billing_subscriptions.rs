use std::collections::BTreeSet;

use autharie_domain::{
    CoreError,
    billing::{
        BillingError, BillingPeriod, Money, Subscription, SubscriptionTerms, TierCode,
        ports::SubscriptionRepository,
    },
    deployments::DeploymentId,
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::billing::PostgresSubscriptionRepository;
use chrono::{DateTime, TimeZone, Utc};
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

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
}

fn terms(price: i64) -> SubscriptionTerms {
    SubscriptionTerms {
        tier: TierCode::legacy(Offer::Standard),
        monthly_price: Money::from_minor(price),
        offer: Offer::Standard,
        features: BTreeSet::from([IamFeature::Mfa, IamFeature::Branding]),
    }
}

fn organisation() -> OrganisationId {
    OrganisationId(Uuid::new_v4())
}

fn deployment() -> DeploymentId {
    DeploymentId(Uuid::new_v4())
}

fn deployments_in_storage_order<const N: usize>() -> [DeploymentId; N] {
    let mut ids = [(); N].map(|()| Uuid::new_v4());
    ids.sort();
    ids.map(DeploymentId)
}

fn october(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap()
}

fn december(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 12, day, 0, 0, 0).unwrap()
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, map_err, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

#[tokio::test]
async fn a_second_open_subscription_of_one_deployment_is_refused() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let organisation = organisation();
        let deployment = deployment();

        subscriptions
            .open(&Subscription::open(
                organisation,
                deployment,
                terms(0),
                at(1, 0),
            ))
            .await?;

        Ok(subscriptions
            .open(&Subscription::open(
                organisation,
                deployment,
                terms(4900),
                at(5, 0),
            ))
            .await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(
                message.contains("billing_subscriptions_one_at_a_time"),
                "{message}"
            );
        }
        other => panic!("expected the exclusion constraint to refuse it, got {other:?}"),
    }
}

#[tokio::test]
async fn a_subscription_overlapping_a_closed_one_is_refused() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let organisation = organisation();
        let deployment = deployment();

        let first = Subscription::open(organisation, deployment, terms(0), at(1, 0));
        subscriptions.open(&first).await?;
        subscriptions.close(&first.close(at(10, 0))?).await?;

        Ok(subscriptions
            .open(&Subscription::open(
                organisation,
                deployment,
                terms(4900),
                at(9, 0),
            ))
            .await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(
                message.contains("billing_subscriptions_one_at_a_time"),
                "{message}"
            );
        }
        other => panic!("expected the exclusion constraint to refuse it, got {other:?}"),
    }
}

#[tokio::test]
async fn a_successor_opened_at_the_instant_its_predecessor_closes_is_accepted() {
    let _pool = pool_or_skip!();

    let latest = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let first = Subscription::open(organisation(), deployment(), terms(0), at(1, 0));
        subscriptions.open(&first).await?;

        let closed = first.close(at(10, 0))?;
        subscriptions.close(&closed).await?;

        let successor = closed.successor(terms(4900), at(10, 0));
        subscriptions.open(&successor).await?;

        subscriptions
            .latest_for(&successor.deployment_id())
            .await
            .map(|latest| (latest, successor))
    })
    .await
    .expect("the database is set");

    let (latest, successor) = latest;
    assert_eq!(latest, Some(successor));
}

#[tokio::test]
async fn a_subscription_changed_the_instant_it_opened_leaves_its_successor_as_the_latest() {
    let _pool = pool_or_skip!();

    let latest = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let first = Subscription::open(organisation(), deployment(), terms(0), at(1, 0));
        subscriptions.open(&first).await?;

        let closed = first.close(at(1, 0))?;
        subscriptions.close(&closed).await?;

        let successor = closed.successor(terms(4900), at(1, 0));
        subscriptions.open(&successor).await?;

        subscriptions
            .latest_for(&successor.deployment_id())
            .await
            .map(|latest| (latest, successor))
    })
    .await
    .expect("the database is set");

    let (latest, successor) = latest;
    assert_eq!(latest, Some(successor));
}

#[tokio::test]
async fn two_deployments_each_hold_their_own_open_subscription() {
    let _pool = pool_or_skip!();

    let (first, second, found) = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let organisation = organisation();
        let first = Subscription::open(organisation, deployment(), terms(0), at(1, 0));
        let second = Subscription::open(organisation, deployment(), terms(0), at(1, 0));

        subscriptions.open(&first).await?;
        subscriptions.open(&second).await?;

        let found = (
            subscriptions.latest_for(&first.deployment_id()).await?,
            subscriptions.latest_for(&second.deployment_id()).await?,
        );
        Ok((first, second, found))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, (Some(first), Some(second)));
}

#[tokio::test]
async fn closing_stores_the_end_and_a_second_close_names_the_subscription() {
    let _pool = pool_or_skip!();

    let (latest, second_close, closed) = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let open = Subscription::open(organisation(), deployment(), terms(4900), at(1, 0));
        subscriptions.open(&open).await?;

        let closed = open.close(at(8, 12))?;
        subscriptions.close(&closed).await?;

        let latest = subscriptions.latest_for(&closed.deployment_id()).await?;
        let second_close = subscriptions.close(&closed).await;

        Ok((latest, second_close, closed))
    })
    .await
    .expect("the database is set");

    assert_eq!(latest, Some(closed.clone()));
    assert!(matches!(
        second_close,
        Err(CoreError::Billing(BillingError::SubscriptionClosed { id })) if id == closed.id().0
    ));
}

#[tokio::test]
async fn closing_a_subscription_that_is_still_open_in_the_domain_is_an_error() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let open = Subscription::open(organisation(), deployment(), terms(4900), at(1, 0));
        subscriptions.open(&open).await?;

        Ok(subscriptions.close(&open).await)
    })
    .await
    .expect("the database is set");

    assert!(
        matches!(outcome, Err(CoreError::InternalError(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn the_latest_subscription_is_the_one_that_started_last() {
    let _pool = pool_or_skip!();

    let (latest, third) = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let first = Subscription::open(organisation(), deployment(), terms(0), at(1, 0));
        subscriptions.open(&first).await?;

        let first = first.close(at(4, 0))?;
        subscriptions.close(&first).await?;

        let second = first.successor(terms(4900), at(4, 0));
        subscriptions.open(&second).await?;
        let second = second.close(at(9, 0))?;
        subscriptions.close(&second).await?;

        let third = second.successor(terms(9900), at(9, 0));
        subscriptions.open(&third).await?;

        let latest = subscriptions.latest_for(&third.deployment_id()).await?;
        Ok((latest, third))
    })
    .await
    .expect("the database is set");

    assert_eq!(latest, Some(third));
}

#[tokio::test]
async fn a_deployment_without_a_subscription_has_no_latest_one() {
    let _pool = pool_or_skip!();

    let latest = scratch(async |tx| {
        PostgresSubscriptionRepository::new(&tx)
            .latest_for(&deployment())
            .await
    })
    .await
    .expect("the database is set");

    assert_eq!(latest, None);
}

#[tokio::test]
async fn the_subscriptions_of_a_period_are_those_overlapping_it_in_deployment_then_start_order() {
    let _pool = pool_or_skip!();

    let november = BillingPeriod::new(2026, 11).unwrap();

    let (listed, expected) = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let organisation = organisation();
        let [early, middle, late] = deployments_in_storage_order();

        let october_to_mid_november =
            Subscription::open(organisation, early, terms(0), october(1)).close(at(10, 0))?;
        let mid_november_on = october_to_mid_november.successor(terms(4900), at(10, 0));
        let ended_at_the_period_start =
            Subscription::open(organisation, middle, terms(0), october(1)).close(at(1, 0))?;
        let december_on = Subscription::open(organisation, middle, terms(4900), december(1));
        let all_november = Subscription::open(organisation, late, terms(4900), at(1, 0));
        let someone_else =
            Subscription::open(self::organisation(), deployment(), terms(0), at(1, 0));

        for subscription in [
            &october_to_mid_november,
            &mid_november_on,
            &december_on,
            &ended_at_the_period_start,
            &all_november,
            &someone_else,
        ] {
            subscriptions.open(subscription).await?;
        }

        let listed = subscriptions
            .list_in_period(&organisation, november)
            .await?;
        Ok((
            listed,
            vec![october_to_mid_november, mid_november_on, all_november],
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(listed, expected);
}

#[tokio::test]
async fn only_organisations_with_a_paid_subscription_in_the_period_are_billable() {
    let _pool = pool_or_skip!();

    let november = BillingPeriod::new(2026, 11).unwrap();

    let (billable, paying, paying_twice, free, ended_before, starts_after) = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let paying = organisation();
        let paying_twice = organisation();
        let free = organisation();
        let ended_before = organisation();
        let starts_after = organisation();

        let paid_here = Subscription::open(paying, deployment(), terms(4900), at(3, 0));
        let paid_there = Subscription::open(paying_twice, deployment(), terms(4900), at(3, 0));
        let paid_again = Subscription::open(paying_twice, deployment(), terms(9900), at(4, 0));
        let free_here = Subscription::open(free, deployment(), terms(0), at(3, 0));
        let paid_in_october =
            Subscription::open(ended_before, deployment(), terms(4900), october(1))
                .close(at(1, 0))?;
        let paid_in_december = Subscription::open(
            starts_after,
            deployment(),
            terms(4900),
            Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap(),
        );

        for subscription in [
            &paid_here,
            &paid_there,
            &paid_again,
            &free_here,
            &paid_in_october,
            &paid_in_december,
        ] {
            subscriptions.open(subscription).await?;
        }

        let billable = subscriptions.organisations_billable_in(november).await?;
        Ok((
            billable,
            paying,
            paying_twice,
            free,
            ended_before,
            starts_after,
        ))
    })
    .await
    .expect("the database is set");

    assert!(billable.contains(&paying));
    assert_eq!(
        billable.iter().filter(|id| **id == paying_twice).count(),
        1,
        "an organisation is listed once however many paid subscriptions it holds"
    );
    assert!(!billable.contains(&free));
    assert!(!billable.contains(&ended_before));
    assert!(!billable.contains(&starts_after));
}

#[tokio::test]
async fn the_open_billable_subscriptions_leave_out_closed_and_free_ones() {
    let _pool = pool_or_skip!();

    let (listed, expected) = scratch(async |tx| {
        let subscriptions = PostgresSubscriptionRepository::new(&tx);
        let organisation = organisation();

        let [first, second, third] = deployments_in_storage_order();

        let paid = Subscription::open(organisation, first, terms(4900), at(3, 0));
        let free = Subscription::open(organisation, second, terms(0), at(3, 0));
        let closed =
            Subscription::open(organisation, third, terms(4900), at(3, 0)).close(at(6, 0))?;
        let elsewhere =
            Subscription::open(self::organisation(), deployment(), terms(4900), at(3, 0));

        for subscription in [&paid, &free, &closed, &elsewhere] {
            subscriptions.open(subscription).await?;
        }

        let listed = subscriptions.list_open_billable(&organisation).await?;
        Ok((listed, vec![paid]))
    })
    .await
    .expect("the database is set");

    assert_eq!(listed, expected);
}

#[tokio::test]
async fn a_stored_offer_the_domain_does_not_know_is_a_database_error_naming_the_column() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let deployment = deployment();
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO billing_subscriptions \
                 (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, \
                  features, starts_at) \
                 VALUES ($1, $2, $3, 'legacy-standard', 0, 'bogus', '{}', $4)",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(deployment.0)
            .bind(at(1, 0))
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        Ok(PostgresSubscriptionRepository::new(&tx)
            .latest_for(&deployment)
            .await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(message.contains("billing_subscriptions.offer"), "{message}");
        }
        other => panic!("expected a database error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_stored_feature_the_domain_does_not_know_is_a_database_error_naming_the_column() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let deployment = deployment();
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO billing_subscriptions \
                 (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, \
                  features, starts_at) \
                 VALUES ($1, $2, $3, 'legacy-standard', 0, 'standard', '{bogus}', $4)",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(deployment.0)
            .bind(at(1, 0))
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        Ok(PostgresSubscriptionRepository::new(&tx)
            .latest_for(&deployment)
            .await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(
                message.contains("billing_subscriptions.features"),
                "{message}"
            );
        }
        other => panic!("expected a database error, got {other:?}"),
    }
}
