use std::time::Duration;

use autharie_domain::{
    CoreError,
    billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingName, BillingPeriod, PaymentMethodStatus, ProviderCustomerId, ProviderInvoice,
        ProviderInvoiceId, Statement, StatementId, StatementParts, StatementStatus,
        ports::{BillingAccountRepository, StatementRepository},
    },
    organisation::OrganisationId,
};
use autharie_persistence::with_tx;
use autharie_postgres::billing::{PostgresBillingAccountRepository, PostgresStatementRepository};
use chrono::{TimeZone, Utc};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::{sync::oneshot, time::timeout};
use uuid::Uuid;

const PATIENCE: Duration = Duration::from_millis(400);

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

#[derive(Clone, Copy)]
enum Read {
    ByKey,
    BySecondKey,
}

fn details() -> BillingDetails {
    BillingDetails {
        name: BillingName::parse("Acme SAS").unwrap(),
        email: BillingEmail::parse("billing@acme.test").unwrap(),
        address: BillingAddress::parse(BillingAddressInput {
            line1: "1 rue de la Paix",
            line2: None,
            postal_code: "75002",
            city: "Paris",
            country: "fr",
        })
        .unwrap(),
        vat_number: None,
        siren: None,
    }
}

async fn a_second_reader_of_an_account_waits_for_the_first_to_commit(read: Read) {
    let Some(pool) = pool().await else {
        return;
    };
    let organisation = OrganisationId(Uuid::new_v4());
    let customer = ProviderCustomerId::parse(&format!("cus_{}", Uuid::new_v4().simple())).unwrap();
    let seeded = BillingAccount {
        customer: Some(customer.clone()),
        payment_method: PaymentMethodStatus::Missing,
        ..BillingAccount::new(organisation, details())
    };
    with_tx(&pool, map_err, async |tx| {
        PostgresBillingAccountRepository::new(&tx)
            .save(&seeded)
            .await
    })
    .await
    .expect("the account is committed");

    let (locked_tx, locked_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();

    let first = tokio::spawn({
        let pool = pool.clone();
        async move {
            with_tx(&pool, map_err, async |tx| {
                let accounts = PostgresBillingAccountRepository::new(&tx);
                let mut account = accounts
                    .find(&organisation)
                    .await?
                    .expect("the seeded account");
                locked_tx.send(()).expect("the test is listening");
                release_rx.await.expect("the test lets go");

                account.payment_method = PaymentMethodStatus::Valid;
                accounts.save(&account).await
            })
            .await
        }
    });
    locked_rx.await.expect("the first writer holds the row");

    let mut second = tokio::spawn({
        let pool = pool.clone();
        let customer = customer.clone();
        async move {
            with_tx(&pool, map_err, async |tx| {
                let accounts = PostgresBillingAccountRepository::new(&tx);
                match read {
                    Read::ByKey => accounts.find(&organisation).await,
                    Read::BySecondKey => accounts.find_by_customer(&customer).await,
                }
            })
            .await
        }
    });

    assert!(
        timeout(PATIENCE, &mut second).await.is_err(),
        "the second reader answered while the first still held the account"
    );

    release_tx.send(()).expect("the first writer is waiting");
    first
        .await
        .expect("the first writer ran")
        .expect("the first writer committed");
    let read = second
        .await
        .expect("the second reader ran")
        .expect("the second reader answered")
        .expect("the account is still there");

    assert_eq!(read.payment_method, PaymentMethodStatus::Valid);

    sqlx::query("DELETE FROM billing_accounts WHERE organisation_id = $1")
        .bind(organisation.0)
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn a_second_reader_of_an_account_by_organisation_waits_for_the_first_to_commit() {
    a_second_reader_of_an_account_waits_for_the_first_to_commit(Read::ByKey).await;
}

#[tokio::test]
async fn a_second_reader_of_an_account_by_customer_waits_for_the_first_to_commit() {
    a_second_reader_of_an_account_waits_for_the_first_to_commit(Read::BySecondKey).await;
}

async fn a_second_reader_of_a_statement_waits_for_the_first_to_commit(read: Read) {
    let Some(pool) = pool().await else {
        return;
    };
    let at = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
    let invoice = ProviderInvoice {
        id: ProviderInvoiceId::parse(&format!("in_{}", Uuid::new_v4().simple())).unwrap(),
        hosted_url: "https://pay.test/in_lock".to_string(),
    };
    let seeded = Statement::from(StatementParts {
        id: StatementId::new(),
        organisation_id: OrganisationId(Uuid::new_v4()),
        period: BillingPeriod::new(2026, 11).unwrap(),
        lines: Vec::new(),
        status: StatementStatus::Sent,
        invoice: Some(invoice.clone()),
        created_at: at,
        updated_at: at,
    });
    let id = seeded.id();
    with_tx(&pool, map_err, async |tx| {
        assert!(
            PostgresStatementRepository::new(&tx)
                .insert(&seeded)
                .await?
        );
        Ok(())
    })
    .await
    .expect("the statement is committed");

    let (locked_tx, locked_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();

    let first = tokio::spawn({
        let pool = pool.clone();
        async move {
            with_tx(&pool, map_err, async |tx| {
                let statements = PostgresStatementRepository::new(&tx);
                let mut statement = statements.find(&id).await?.expect("the seeded statement");
                locked_tx.send(()).expect("the test is listening");
                release_rx.await.expect("the test lets go");

                statement.advance(StatementStatus::Paid, at)?;
                statements.update(&statement).await
            })
            .await
        }
    });
    locked_rx.await.expect("the first writer holds the row");

    let mut second = tokio::spawn({
        let pool = pool.clone();
        let invoice = invoice.id.clone();
        async move {
            with_tx(&pool, map_err, async |tx| {
                let statements = PostgresStatementRepository::new(&tx);
                match read {
                    Read::ByKey => statements.find(&id).await,
                    Read::BySecondKey => statements.find_by_invoice(&invoice).await,
                }
            })
            .await
        }
    });

    assert!(
        timeout(PATIENCE, &mut second).await.is_err(),
        "the second reader answered while the first still held the statement"
    );

    release_tx.send(()).expect("the first writer is waiting");
    first
        .await
        .expect("the first writer ran")
        .expect("the first writer committed");
    let read = second
        .await
        .expect("the second reader ran")
        .expect("the second reader answered")
        .expect("the statement is still there");

    assert_eq!(read.status(), StatementStatus::Paid);

    sqlx::query("DELETE FROM billing_statements WHERE id = $1")
        .bind(id.0)
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn a_second_reader_of_a_statement_by_id_waits_for_the_first_to_commit() {
    a_second_reader_of_a_statement_waits_for_the_first_to_commit(Read::ByKey).await;
}

#[tokio::test]
async fn a_second_reader_of_a_statement_by_invoice_waits_for_the_first_to_commit() {
    a_second_reader_of_a_statement_waits_for_the_first_to_commit(Read::BySecondKey).await;
}
