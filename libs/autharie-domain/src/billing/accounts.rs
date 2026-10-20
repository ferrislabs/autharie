use crate::{
    CoreError,
    billing::{
        BillingAccount, BillingDetails, BillingError,
        ports::{BillingAccountRepository, PaymentProvider},
    },
    organisation::OrganisationId,
};

pub async fn keep_details<A: BillingAccountRepository, P: PaymentProvider>(
    accounts: &A,
    provider: Option<&P>,
    organisation_id: OrganisationId,
    details: BillingDetails,
) -> Result<BillingAccount, CoreError> {
    let mut account = match accounts.find(&organisation_id).await? {
        Some(existing) => BillingAccount {
            details,
            ..existing
        },
        None => BillingAccount::new(organisation_id, details),
    };

    if let Some(provider) = provider {
        match account.customer.as_ref() {
            Some(customer) => provider.update_customer(customer, &account.details).await?,
            None => {
                let customer = provider
                    .create_customer(&organisation_id, &account.details)
                    .await?;
                account.customer = Some(customer);
            }
        }
    }

    accounts.save(&account).await?;

    Ok(account)
}

pub async fn open_payment_method_session<A: BillingAccountRepository, P: PaymentProvider>(
    accounts: &A,
    provider: &P,
    organisation_id: OrganisationId,
    return_url: &str,
) -> Result<String, CoreError> {
    let mut account = accounts
        .find(&organisation_id)
        .await?
        .ok_or(BillingError::NoBillingAccount)?;

    if account.customer.is_none() {
        let customer = provider
            .create_customer(&organisation_id, &account.details)
            .await?;
        account.customer = Some(customer);
        accounts.save(&account).await?;
    }

    match account.customer.as_ref() {
        Some(customer) => provider.payment_method_session(customer, return_url).await,
        None => Err(BillingError::NoBillingAccount.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mockall::predicate::eq;

    use super::*;
    use crate::billing::{
        BillingAddress, BillingAddressInput, BillingEmail, BillingName, PaymentMethodStatus,
        ProviderCustomerId,
        ports::{MockBillingAccountRepository, MockPaymentProvider},
    };

    #[derive(Clone, Default)]
    struct Saved(Arc<Mutex<Vec<BillingAccount>>>);

    impl Saved {
        fn note(&self, account: &BillingAccount) {
            self.0.lock().expect("not poisoned").push(account.clone());
        }

        fn all(&self) -> Vec<BillingAccount> {
            self.0.lock().expect("not poisoned").clone()
        }
    }

    fn details(name: &str) -> BillingDetails {
        BillingDetails {
            name: BillingName::parse(name).unwrap(),
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

    fn customer(id: &str) -> ProviderCustomerId {
        ProviderCustomerId::parse(id).unwrap()
    }

    fn account(
        organisation_id: OrganisationId,
        customer_id: Option<&str>,
        payment_method: PaymentMethodStatus,
    ) -> BillingAccount {
        BillingAccount {
            customer: customer_id.map(customer),
            payment_method,
            ..BillingAccount::new(organisation_id, details("Old Name"))
        }
    }

    fn accounts_holding(
        held: Option<BillingAccount>,
        saved: &Saved,
    ) -> MockBillingAccountRepository {
        let mut mock = MockBillingAccountRepository::new();
        mock.expect_find().returning(move |_| {
            let held = held.clone();
            Box::pin(async move { Ok(held) })
        });
        let saved = saved.clone();
        mock.expect_save().returning(move |account| {
            saved.note(account);
            Box::pin(async { Ok(()) })
        });
        mock
    }

    fn strict_provider() -> MockPaymentProvider {
        MockPaymentProvider::new()
    }

    fn no_provider() -> Option<&'static MockPaymentProvider> {
        None
    }

    #[tokio::test]
    async fn without_a_provider_a_first_account_is_stored_with_no_customer() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(None, &saved);

        let kept = keep_details(&accounts, no_provider(), organisation_id, details("Acme"))
            .await
            .unwrap();

        assert_eq!(kept.organisation_id, organisation_id);
        assert_eq!(kept.details, details("Acme"));
        assert_eq!(kept.customer, None);
        assert_eq!(kept.payment_method, PaymentMethodStatus::Missing);
        assert_eq!(saved.all(), vec![kept]);
    }

    #[tokio::test]
    async fn new_details_replace_the_old_and_keep_the_customer_and_the_payment_method() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(
            Some(account(
                organisation_id,
                Some("cus_1"),
                PaymentMethodStatus::Valid,
            )),
            &saved,
        );

        let kept = keep_details(&accounts, no_provider(), organisation_id, details("Acme"))
            .await
            .unwrap();

        assert_eq!(kept.details, details("Acme"));
        assert_eq!(kept.customer, Some(customer("cus_1")));
        assert_eq!(kept.payment_method, PaymentMethodStatus::Valid);
        assert_eq!(saved.all(), vec![kept]);
    }

    #[tokio::test]
    async fn an_existing_customer_is_updated_not_created_again() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(
            Some(account(
                organisation_id,
                Some("cus_1"),
                PaymentMethodStatus::Failing,
            )),
            &saved,
        );
        let mut provider = strict_provider();
        provider
            .expect_update_customer()
            .withf(|customer, details| {
                customer.as_str() == "cus_1" && details.name.as_str() == "Acme"
            })
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(()) }));

        let kept = keep_details(&accounts, Some(&provider), organisation_id, details("Acme"))
            .await
            .unwrap();

        assert_eq!(kept.customer, Some(customer("cus_1")));
        assert_eq!(kept.payment_method, PaymentMethodStatus::Failing);
        assert_eq!(saved.all(), vec![kept]);
    }

    #[tokio::test]
    async fn a_missing_customer_is_created_and_stored() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(None, &saved);
        let mut provider = strict_provider();
        provider
            .expect_create_customer()
            .withf(move |id, details| *id == organisation_id && details.name.as_str() == "Acme")
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(customer("cus_new")) }));

        let kept = keep_details(&accounts, Some(&provider), organisation_id, details("Acme"))
            .await
            .unwrap();

        assert_eq!(kept.customer, Some(customer("cus_new")));
        assert_eq!(kept.payment_method, PaymentMethodStatus::Missing);
        assert_eq!(saved.all(), vec![kept]);
    }

    #[tokio::test]
    async fn an_account_stored_without_a_customer_gets_one_when_the_provider_is_there() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(
            Some(account(organisation_id, None, PaymentMethodStatus::Missing)),
            &saved,
        );
        let mut provider = strict_provider();
        provider
            .expect_create_customer()
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(customer("cus_late")) }));

        let kept = keep_details(&accounts, Some(&provider), organisation_id, details("Acme"))
            .await
            .unwrap();

        assert_eq!(kept.customer, Some(customer("cus_late")));
        assert_eq!(saved.all(), vec![kept]);
    }

    #[tokio::test]
    async fn a_provider_failure_stores_nothing() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(None, &saved);
        let mut provider = strict_provider();
        provider.expect_create_customer().returning(|_, _| {
            Box::pin(async {
                Err(BillingError::Provider {
                    reason: "down".to_string(),
                }
                .into())
            })
        });

        let refused =
            keep_details(&accounts, Some(&provider), organisation_id, details("Acme")).await;

        assert!(matches!(
            refused,
            Err(CoreError::Billing(BillingError::Provider { .. }))
        ));
        assert!(saved.all().is_empty());
    }

    #[tokio::test]
    async fn a_session_needs_an_account() {
        let saved = Saved::default();
        let accounts = accounts_holding(None, &saved);

        let refused = open_payment_method_session(
            &accounts,
            &strict_provider(),
            OrganisationId::new(),
            "https://console.test/billing",
        )
        .await;

        assert!(matches!(
            refused,
            Err(CoreError::Billing(BillingError::NoBillingAccount))
        ));
        assert!(saved.all().is_empty());
    }

    #[tokio::test]
    async fn a_session_is_opened_for_the_customer_on_file() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(
            Some(account(
                organisation_id,
                Some("cus_1"),
                PaymentMethodStatus::Missing,
            )),
            &saved,
        );
        let mut provider = strict_provider();
        provider
            .expect_payment_method_session()
            .with(eq(customer("cus_1")), eq("https://console.test/billing"))
            .times(1)
            .returning(|_, _| Box::pin(async { Ok("https://checkout.test/s/1".to_string()) }));

        let url = open_payment_method_session(
            &accounts,
            &provider,
            organisation_id,
            "https://console.test/billing",
        )
        .await
        .unwrap();

        assert_eq!(url, "https://checkout.test/s/1");
        assert!(saved.all().is_empty());
    }

    #[tokio::test]
    async fn a_session_creates_and_stores_the_customer_it_lacks() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(
            Some(account(organisation_id, None, PaymentMethodStatus::Missing)),
            &saved,
        );
        let mut provider = strict_provider();
        provider
            .expect_create_customer()
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(customer("cus_new")) }));
        provider
            .expect_payment_method_session()
            .with(eq(customer("cus_new")), eq("https://console.test/billing"))
            .times(1)
            .returning(|_, _| Box::pin(async { Ok("https://checkout.test/s/2".to_string()) }));

        let url = open_payment_method_session(
            &accounts,
            &provider,
            organisation_id,
            "https://console.test/billing",
        )
        .await
        .unwrap();

        assert_eq!(url, "https://checkout.test/s/2");
        let stored = saved.all();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].customer, Some(customer("cus_new")));
        assert_eq!(stored[0].details, details("Old Name"));
    }

    #[tokio::test]
    async fn a_session_the_provider_cannot_open_is_the_providers_failure() {
        let organisation_id = OrganisationId::new();
        let saved = Saved::default();
        let accounts = accounts_holding(
            Some(account(
                organisation_id,
                Some("cus_1"),
                PaymentMethodStatus::Missing,
            )),
            &saved,
        );
        let mut provider = strict_provider();
        provider.expect_payment_method_session().returning(|_, _| {
            Box::pin(async {
                Err(BillingError::Provider {
                    reason: "down".to_string(),
                }
                .into())
            })
        });

        let refused =
            open_payment_method_session(&accounts, &provider, organisation_id, "https://x.test")
                .await;

        assert!(matches!(
            refused,
            Err(CoreError::Billing(BillingError::Provider { .. }))
        ));
    }
}
