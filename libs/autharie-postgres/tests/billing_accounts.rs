use autharie_domain::{
    CoreError,
    billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingName, PaymentMethodStatus, ProviderCustomerId, Siren, VatNumber,
        ports::BillingAccountRepository,
    },
    organisation::OrganisationId,
};
use autharie_persistence::{SharedTx, in_scratch_tx};
use autharie_postgres::billing::PostgresBillingAccountRepository;
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

async fn scratch<T>(work: impl AsyncFnOnce(SharedTx<'_>) -> Result<T, CoreError>) -> Option<T> {
    let pool = pool().await?;

    Some(
        in_scratch_tx(&pool, map_err, work)
            .await
            .expect("the scratch transaction ran"),
    )
}

fn complete_details() -> BillingDetails {
    BillingDetails {
        name: BillingName::parse("Acme SAS").unwrap(),
        email: BillingEmail::parse("billing@acme.test").unwrap(),
        address: BillingAddress::parse(BillingAddressInput {
            line1: "1 rue de la Paix",
            line2: Some("Bâtiment B"),
            postal_code: "75002",
            city: "Paris",
            country: "fr",
        })
        .unwrap(),
        vat_number: Some(VatNumber::parse("FR 12 345 678 901").unwrap()),
        siren: Some(Siren::parse("123 456 789").unwrap()),
    }
}

fn minimal_details() -> BillingDetails {
    BillingDetails {
        name: BillingName::parse("Globex").unwrap(),
        email: BillingEmail::parse("accounts@globex.test").unwrap(),
        address: BillingAddress::parse(BillingAddressInput {
            line1: "2 Hauptstrasse",
            line2: None,
            postal_code: "10115",
            city: "Berlin",
            country: "DE",
        })
        .unwrap(),
        vat_number: None,
        siren: None,
    }
}

fn customer(raw: &str) -> ProviderCustomerId {
    ProviderCustomerId::parse(raw).unwrap()
}

#[tokio::test]
async fn an_account_is_saved_and_read_back_with_every_field() {
    let _pool = pool_or_skip!();

    let (found, account) = scratch(async |tx| {
        let accounts = PostgresBillingAccountRepository::new(&tx);
        let account = BillingAccount {
            customer: Some(customer("cus_full")),
            payment_method: PaymentMethodStatus::Valid,
            ..BillingAccount::new(OrganisationId(Uuid::new_v4()), complete_details())
        };
        accounts.save(&account).await?;

        Ok((accounts.find(&account.organisation_id).await?, account))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(account));
}

#[tokio::test]
async fn an_account_without_optional_fields_or_customer_is_read_back() {
    let _pool = pool_or_skip!();

    let (found, account) = scratch(async |tx| {
        let accounts = PostgresBillingAccountRepository::new(&tx);
        let account = BillingAccount::new(OrganisationId(Uuid::new_v4()), minimal_details());
        accounts.save(&account).await?;

        Ok((accounts.find(&account.organisation_id).await?, account))
    })
    .await
    .expect("the database is set");

    assert_eq!(found.as_ref(), Some(&account));
    assert_eq!(account.payment_method, PaymentMethodStatus::Missing);
    assert_eq!(account.customer, None);
}

#[tokio::test]
async fn saving_again_replaces_every_column_of_the_account() {
    let _pool = pool_or_skip!();

    let (found, replaced) = scratch(async |tx| {
        let accounts = PostgresBillingAccountRepository::new(&tx);
        let organisation = OrganisationId(Uuid::new_v4());

        accounts
            .save(&BillingAccount {
                customer: Some(customer("cus_before")),
                payment_method: PaymentMethodStatus::Failing,
                ..BillingAccount::new(organisation, complete_details())
            })
            .await?;

        let replaced = BillingAccount {
            customer: Some(customer("cus_after")),
            payment_method: PaymentMethodStatus::Valid,
            ..BillingAccount::new(organisation, minimal_details())
        };
        accounts.save(&replaced).await?;

        Ok((accounts.find(&organisation).await?, replaced))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(replaced));
}

#[tokio::test]
async fn an_account_is_found_by_its_customer() {
    let _pool = pool_or_skip!();

    let (found, unknown, account) = scratch(async |tx| {
        let accounts = PostgresBillingAccountRepository::new(&tx);
        let account = BillingAccount {
            customer: Some(customer("cus_lookup")),
            payment_method: PaymentMethodStatus::Valid,
            ..BillingAccount::new(OrganisationId(Uuid::new_v4()), complete_details())
        };
        accounts.save(&account).await?;

        Ok((
            accounts.find_by_customer(&customer("cus_lookup")).await?,
            accounts.find_by_customer(&customer("cus_nobody")).await?,
            account,
        ))
    })
    .await
    .expect("the database is set");

    assert_eq!(found, Some(account));
    assert_eq!(unknown, None);
}

#[tokio::test]
async fn an_organisation_without_an_account_has_none() {
    let _pool = pool_or_skip!();

    let found = scratch(async |tx| {
        PostgresBillingAccountRepository::new(&tx)
            .find(&OrganisationId(Uuid::new_v4()))
            .await
    })
    .await
    .expect("the database is set");

    assert_eq!(found, None);
}

#[tokio::test]
async fn a_customer_belongs_to_one_account() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let accounts = PostgresBillingAccountRepository::new(&tx);
        let first = BillingAccount {
            customer: Some(customer("cus_shared")),
            ..BillingAccount::new(OrganisationId(Uuid::new_v4()), complete_details())
        };
        let second = BillingAccount {
            customer: Some(customer("cus_shared")),
            ..BillingAccount::new(OrganisationId(Uuid::new_v4()), minimal_details())
        };
        accounts.save(&first).await?;

        Ok(accounts.save(&second).await)
    })
    .await
    .expect("the database is set");

    assert!(
        matches!(outcome, Err(CoreError::DatabaseError { .. })),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_stored_value_the_domain_refuses_is_a_database_error_naming_the_column() {
    let _pool = pool_or_skip!();

    let outcome = scratch(async |tx| {
        let organisation = OrganisationId(Uuid::new_v4());
        {
            let mut guard = tx.lock().await;
            sqlx::query(
                "INSERT INTO billing_accounts \
                 (organisation_id, name, email, address_line1, postal_code, city, country) \
                 VALUES ($1, 'Acme', 'not an email', '1 rue', '75002', 'Paris', 'FR')",
            )
            .bind(organisation.0)
            .execute(&mut ***guard)
            .await
            .map_err(map_err)?;
        }

        Ok(PostgresBillingAccountRepository::new(&tx)
            .find(&organisation)
            .await)
    })
    .await
    .expect("the database is set");

    match outcome {
        Err(CoreError::DatabaseError { message }) => {
            assert!(message.contains("billing_accounts.email"), "{message}");
        }
        other => panic!("expected a database error, got {other:?}"),
    }
}
