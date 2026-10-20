use std::collections::BTreeSet;

use autharie_domain::{
    CoreError,
    billing::{
        BillingPeriod, Money, ProviderEventId, ProviderInvoice, ProviderInvoiceId, Statement,
        StatementId, StatementLine, StatementParts, StatementStatus, Subscription,
        SubscriptionTerms, TierCode,
        ports::{ProviderEventLog, StatementRepository, SubscriptionRepository},
    },
    deployments::DeploymentId,
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::billing::{
    PostgresProviderEventRepository, PostgresStatementRepository, PostgresSubscriptionRepository,
};
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

fn at(month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap()
}

fn period(month: u32) -> BillingPeriod {
    BillingPeriod::new(2026, month).unwrap()
}

fn terms() -> SubscriptionTerms {
    SubscriptionTerms {
        tier: TierCode::legacy(Offer::Standard),
        monthly_price: Money::from_minor(4900),
        offer: Offer::Standard,
        features: BTreeSet::from([IamFeature::Mfa]),
    }
}

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, map_err, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

async fn a_subscription(
    tx: &SharedTx<'_>,
    organisation: OrganisationId,
) -> Result<Subscription, CoreError> {
    let subscription = Subscription::open(
        organisation,
        DeploymentId(Uuid::new_v4()),
        terms(),
        at(10, 1),
    );
    PostgresSubscriptionRepository::new(tx)
        .open(&subscription)
        .await?;

    Ok(subscription)
}

fn line_of(subscription: &Subscription, period: BillingPeriod, amount: i64) -> StatementLine {
    StatementLine {
        deployment_id: subscription.deployment_id(),
        subscription_id: subscription.id(),
        tier: subscription.terms().tier.clone(),
        monthly_price: subscription.terms().monthly_price,
        starts_at: period.starts_at(),
        ends_at: period.ends_at(),
        covered_seconds: period.seconds(),
        amount: Money::from_minor(amount),
    }
}

fn statement_of(
    organisation: OrganisationId,
    period: BillingPeriod,
    status: StatementStatus,
    lines: Vec<StatementLine>,
    created_at: DateTime<Utc>,
) -> Statement {
    Statement::from(StatementParts {
        id: StatementId::new(),
        organisation_id: organisation,
        period,
        lines,
        status,
        invoice: None,
        created_at,
        updated_at: created_at,
    })
}

fn invoice(id: &str) -> ProviderInvoice {
    ProviderInvoice {
        id: ProviderInvoiceId::parse(id).unwrap(),
        hosted_url: format!("https://pay.test/{id}"),
    }
}

#[tokio::test]
async fn a_statement_is_stored_with_its_lines_in_order_and_read_back_whole() {
    let _pool = pool_or_skip!();

    let (found, statement) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let first = a_subscription(&tx, organisation).await?;
        let second = a_subscription(&tx, organisation).await?;
        let third = a_subscription(&tx, organisation).await?;
        let statement = statement_of(
            organisation,
            period(11),
            StatementStatus::Held,
            vec![
                line_of(&second, period(11), 2450),
                line_of(&third, period(11), 4900),
                line_of(&first, period(11), 1225),
            ],
            at(12, 1),
        );

        let statements = PostgresStatementRepository::new(&tx);
        assert!(statements.insert(&statement).await?);

        Ok((statements.find(&statement.id()).await?, statement))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(statement));
}

#[tokio::test]
async fn a_second_statement_for_the_same_organisation_and_period_is_refused() {
    let _pool = pool_or_skip!();

    let (second_inserted, kept, intruder, first) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let subscription = a_subscription(&tx, organisation).await?;
        let other_subscription = a_subscription(&tx, organisation).await?;
        let statements = PostgresStatementRepository::new(&tx);

        let first = statement_of(
            organisation,
            period(11),
            StatementStatus::Held,
            vec![
                line_of(&subscription, period(11), 4900),
                line_of(&other_subscription, period(11), 4900),
            ],
            at(12, 1),
        );
        assert!(statements.insert(&first).await?);

        let second = statement_of(
            organisation,
            period(11),
            StatementStatus::Draft,
            vec![line_of(&subscription, period(11), 100)],
            at(12, 2),
        );
        let second_inserted = statements.insert(&second).await?;

        Ok((
            second_inserted,
            statements.find_by_period(&organisation, period(11)).await?,
            statements.find(&second.id()).await?,
            first,
        ))
    })
    .await
    .expect("the database is set");

    assert!(!second_inserted);
    assert_eq!(intruder, None);
    assert_eq!(kept.as_ref(), Some(&first));
    assert_eq!(kept.map(|kept| kept.lines().len()), Some(2));
}

#[tokio::test]
async fn two_organisations_each_get_a_statement_for_the_same_period() {
    let _pool = pool_or_skip!();

    let (first_inserted, second_inserted) = scratch(async |tx| {
        let statements = PostgresStatementRepository::new(&tx);
        let mut inserted = Vec::new();

        for _ in 0..2 {
            let organisation = OrganisationId(Uuid::new_v4());
            let subscription = a_subscription(&tx, organisation).await?;
            inserted.push(
                statements
                    .insert(&statement_of(
                        organisation,
                        period(11),
                        StatementStatus::Held,
                        vec![line_of(&subscription, period(11), 4900)],
                        at(12, 1),
                    ))
                    .await?,
            );
        }

        Ok((inserted[0], inserted[1]))
    })
    .await
    .expect("the database is set");

    assert!(first_inserted && second_inserted);
}

#[tokio::test]
async fn a_statement_without_lines_is_stored_and_read_back() {
    let _pool = pool_or_skip!();

    let (found, statement) = scratch(async |tx| {
        let statements = PostgresStatementRepository::new(&tx);
        let statement = statement_of(
            OrganisationId(Uuid::new_v4()),
            period(11),
            StatementStatus::Zero,
            Vec::new(),
            at(12, 1),
        );
        assert!(statements.insert(&statement).await?);

        Ok((statements.find(&statement.id()).await?, statement))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(statement));
}

#[tokio::test]
async fn an_unknown_statement_is_not_found() {
    let _pool = pool_or_skip!();

    let (by_id, by_period, by_invoice) = scratch(async |tx| {
        let statements = PostgresStatementRepository::new(&tx);

        Ok((
            statements.find(&StatementId::new()).await?,
            statements
                .find_by_period(&OrganisationId(Uuid::new_v4()), period(11))
                .await?,
            statements
                .find_by_invoice(&invoice("in_unknown").id)
                .await?,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!((by_id, by_period, by_invoice), (None, None, None));
}

#[tokio::test]
async fn an_updated_statement_is_found_by_its_invoice() {
    let _pool = pool_or_skip!();

    let (by_invoice, by_id, by_period, sent) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let subscription = a_subscription(&tx, organisation).await?;
        let statements = PostgresStatementRepository::new(&tx);

        let mut statement = statement_of(
            organisation,
            period(11),
            StatementStatus::Draft,
            vec![line_of(&subscription, period(11), 4900)],
            at(12, 1),
        );
        statements.insert(&statement).await?;
        assert_eq!(
            statements
                .find_by_invoice(&ProviderInvoiceId::parse("in_123").unwrap())
                .await?,
            None
        );

        statement.mark_sent(invoice("in_123"), at(12, 3))?;
        statements.update(&statement).await?;

        Ok((
            statements
                .find_by_invoice(&ProviderInvoiceId::parse("in_123").unwrap())
                .await?,
            statements.find(&statement.id()).await?,
            statements.find_by_period(&organisation, period(11)).await?,
            statement,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(sent.status(), StatementStatus::Sent);
    assert_eq!(by_invoice.as_ref(), Some(&sent));
    assert_eq!(by_id.as_ref(), Some(&sent));
    assert_eq!(by_period.as_ref(), Some(&sent));
    assert_eq!(
        by_invoice.and_then(|found| found.invoice().cloned()),
        Some(invoice("in_123"))
    );
}

#[tokio::test]
async fn updating_a_statement_leaves_its_lines_alone() {
    let _pool = pool_or_skip!();

    let (found, paid) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let subscription = a_subscription(&tx, organisation).await?;
        let statements = PostgresStatementRepository::new(&tx);

        let mut statement = statement_of(
            organisation,
            period(11),
            StatementStatus::Draft,
            vec![line_of(&subscription, period(11), 4900)],
            at(12, 1),
        );
        statements.insert(&statement).await?;
        statement.mark_sent(invoice("in_456"), at(12, 3))?;
        statement.advance(StatementStatus::Paid, at(12, 9))?;
        statements.update(&statement).await?;

        Ok((statements.find(&statement.id()).await?, statement))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(paid));
}

#[tokio::test]
async fn an_invoice_belongs_to_one_statement() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let statements = PostgresStatementRepository::new(&tx);
        let mut both = Vec::new();

        for month in [10, 11] {
            let organisation = OrganisationId(Uuid::new_v4());
            let subscription = a_subscription(&tx, organisation).await?;
            let mut statement = statement_of(
                organisation,
                period(month),
                StatementStatus::Draft,
                vec![line_of(&subscription, period(month), 4900)],
                at(12, 1),
            );
            statements.insert(&statement).await?;
            statement.mark_sent(invoice("in_shared"), at(12, 3))?;
            both.push(statement);
        }

        statements.update(&both[0]).await?;
        Ok(statements.update(&both[1]).await)
    })
    .await
    .expect("the database is set");

    assert!(
        matches!(outcome, Err(CoreError::DatabaseError { .. })),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_organisation_lists_its_statements_newest_period_first() {
    let _pool = pool_or_skip!();

    let (listed, expected) = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let subscription = a_subscription(&tx, organisation).await?;
        let statements = PostgresStatementRepository::new(&tx);

        let mut made = Vec::new();
        for month in [10, 12, 11] {
            let statement = statement_of(
                organisation,
                period(month),
                StatementStatus::Held,
                vec![line_of(&subscription, period(month), 4900)],
                at(month, 28),
            );
            statements.insert(&statement).await?;
            made.push(statement);
        }

        let elsewhere = OrganisationId(Uuid::new_v4());
        statements
            .insert(&statement_of(
                elsewhere,
                period(11),
                StatementStatus::Zero,
                Vec::new(),
                at(12, 1),
            ))
            .await?;

        let listed = statements.list_for(&organisation).await?;
        let expected = vec![made[1].clone(), made[2].clone(), made[0].clone()];
        Ok((listed, expected))
    })
    .await
    .expect("the database is set");

    assert_eq!(listed, expected);
}

#[tokio::test]
async fn unsent_statements_are_the_held_and_draft_ones_oldest_period_first() {
    let _pool = pool_or_skip!();

    let (listed, expected) = scratch(async |tx| {
        let statements = PostgresStatementRepository::new(&tx);
        let mut organisations = Vec::new();
        let mut expected_order = Vec::new();

        let plan = [
            (period(12), StatementStatus::Draft, at(12, 5), true),
            (period(10), StatementStatus::Held, at(11, 5), true),
            (period(11), StatementStatus::Paid, at(12, 5), false),
            (period(11), StatementStatus::Zero, at(12, 5), false),
            (period(11), StatementStatus::Sent, at(12, 5), false),
            (period(11), StatementStatus::Held, at(12, 6), true),
            (period(11), StatementStatus::Draft, at(12, 4), true),
        ];

        let mut made = Vec::new();
        for (period, status, created_at, unsent) in plan {
            let organisation = OrganisationId(Uuid::new_v4());
            let subscription = a_subscription(&tx, organisation).await?;
            let statement = statement_of(
                organisation,
                period,
                status,
                vec![line_of(&subscription, period, 4900)],
                created_at,
            );
            statements.insert(&statement).await?;
            organisations.push(organisation);
            made.push((statement, unsent));
        }

        for index in [1, 6, 5, 0] {
            expected_order.push(made[index].0.clone());
        }

        let listed: Vec<Statement> = statements
            .list_unsent()
            .await?
            .into_iter()
            .filter(|statement| organisations.contains(&statement.organisation_id()))
            .collect();

        Ok((listed, expected_order))
    })
    .await
    .expect("the database is set");

    assert_eq!(listed, expected);
}

#[tokio::test]
async fn a_provider_event_is_recorded_once() {
    let _pool = pool_or_skip!();

    let outcomes = scratch(async |tx| {
        let events = PostgresProviderEventRepository::new(&tx);
        let first = ProviderEventId::parse("evt_billing_once").unwrap();
        let other = ProviderEventId::parse("evt_billing_other").unwrap();

        Ok([
            events.record(&first, at(12, 1)).await?,
            events.record(&first, at(12, 2)).await?,
            events.record(&other, at(12, 2)).await?,
            events.record(&first, at(12, 3)).await?,
        ])
    })
    .await
    .expect("the database is set");

    assert_eq!(outcomes, [true, false, true, false]);
}

#[tokio::test]
async fn an_invoice_without_its_hosted_url_is_a_database_error_naming_the_column() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let id = StatementId::new();
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO billing_statements \
                 (id, organisation_id, period, status, total_minor, provider_invoice_id) \
                 VALUES ($1, $2, '2026-11', 'sent', 0, 'in_orphan')",
            )
            .bind(id.0)
            .bind(Uuid::new_v4())
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        Ok(PostgresStatementRepository::new(&tx).find(&id).await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(
                message.contains("billing_statements.hosted_invoice_url"),
                "{message}"
            );
        }
        other => panic!("expected a database error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_stored_line_tier_the_domain_does_not_know_is_a_database_error_naming_the_column() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        let subscription = a_subscription(&tx, organisation).await?;
        let id = StatementId::new();
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO billing_statements \
                 (id, organisation_id, period, status, total_minor) \
                 VALUES ($1, $2, '2026-11', 'held', 0)",
            )
            .bind(id.0)
            .bind(organisation.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
            sqlx::query(
                "INSERT INTO billing_statement_lines \
                 (statement_id, position, deployment_id, subscription_id, tier_code, \
                  monthly_price_minor, starts_at, ends_at, covered_seconds, amount_minor) \
                 VALUES ($1, 0, $2, $3, 'Not A Tier', 0, $4, $5, 0, 0)",
            )
            .bind(id.0)
            .bind(subscription.deployment_id().0)
            .bind(subscription.id().0)
            .bind(at(11, 1))
            .bind(at(12, 1))
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        Ok(PostgresStatementRepository::new(&tx).find(&id).await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(
                message.contains("billing_statement_lines.tier_code"),
                "{message}"
            );
        }
        other => panic!("expected a database error, got {other:?}"),
    }
}
