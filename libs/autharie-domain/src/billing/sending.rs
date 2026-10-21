use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    billing::{
        Statement, StatementStatus,
        ports::{BillingAccountRepository, PaymentProvider, StatementRepository},
    },
};

pub struct StatementSending<A, ST, P> {
    accounts: A,
    statements: ST,
    provider: P,
}

impl<A, ST, P> StatementSending<A, ST, P>
where
    A: BillingAccountRepository,
    ST: StatementRepository,
    P: PaymentProvider,
{
    pub fn new(accounts: A, statements: ST, provider: P) -> Self {
        Self {
            accounts,
            statements,
            provider,
        }
    }

    pub async fn send_current(
        &self,
        statement: Statement,
        now: DateTime<Utc>,
    ) -> Result<Statement, CoreError> {
        match self.statements.find(&statement.id()).await? {
            Some(current) if current.status().is_unsent() => self.send(current, now).await,
            _ => Ok(statement),
        }
    }

    pub async fn send(
        &self,
        mut statement: Statement,
        now: DateTime<Utc>,
    ) -> Result<Statement, CoreError> {
        let status = statement.status();

        if !status.is_unsent() {
            return Ok(statement);
        }

        let Some(account) = self.accounts.find(&statement.organisation_id()).await? else {
            return Ok(statement);
        };
        let Some(customer) = account.customer.as_ref() else {
            return Ok(statement);
        };

        if status == StatementStatus::Held {
            if !account.can_be_invoiced() {
                return Ok(statement);
            }

            statement.advance(StatementStatus::Draft, now)?;
            self.statements.update(&statement).await?;
        }

        let invoice = self.provider.issue_invoice(customer, &statement).await?;
        statement.mark_sent(invoice, now)?;
        self.statements.update(&statement).await?;

        Ok(statement)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::TimeZone;

    use super::*;
    use crate::{
        billing::{
            BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
            BillingError, BillingName, BillingPeriod, Money, PaymentMethodStatus,
            ProviderCustomerId, ProviderInvoice, ProviderInvoiceId, StatementId, StatementLine,
            StatementParts, StatementStatus, SubscriptionId, TierCode,
            ports::{MockBillingAccountRepository, MockPaymentProvider, MockStatementRepository},
        },
        deployments::DeploymentId,
        offers::Offer,
        organisation::OrganisationId,
    };

    type Stored = (StatementStatus, Option<ProviderInvoice>);

    #[derive(Clone, Default)]
    struct Updates(Arc<Mutex<Vec<Stored>>>);

    impl Updates {
        fn note(&self, statement: &Statement) {
            self.0
                .lock()
                .expect("not poisoned")
                .push((statement.status(), statement.invoice().cloned()));
        }

        fn statuses(&self) -> Vec<StatementStatus> {
            self.0
                .lock()
                .expect("not poisoned")
                .iter()
                .map(|(status, _)| *status)
                .collect()
        }
    }

    type Sending = StatementSending<
        MockBillingAccountRepository,
        MockStatementRepository,
        MockPaymentProvider,
    >;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, day, 0, 0, 0).unwrap()
    }

    fn period() -> BillingPeriod {
        BillingPeriod::new(2026, 11).unwrap()
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

    fn account(customer: bool, payment_method: PaymentMethodStatus) -> BillingAccount {
        BillingAccount {
            customer: customer.then(|| ProviderCustomerId::parse("cus_123").unwrap()),
            payment_method,
            ..BillingAccount::new(OrganisationId::new(), details())
        }
    }

    fn statement(status: StatementStatus) -> Statement {
        Statement::from(StatementParts {
            id: StatementId::new(),
            organisation_id: OrganisationId::new(),
            period: period(),
            lines: vec![StatementLine {
                deployment_id: DeploymentId(uuid::Uuid::new_v4()),
                subscription_id: SubscriptionId::new(),
                tier: TierCode::legacy(Offer::Standard),
                monthly_price: Money::from_minor(4900),
                starts_at: period().starts_at(),
                ends_at: period().ends_at(),
                covered_seconds: period().seconds(),
                amount: Money::from_minor(4900),
            }],
            status,
            invoice: None,
            created_at: at(1),
            updated_at: at(1),
        })
    }

    fn moved_to(
        statement: &Statement,
        status: StatementStatus,
        invoice: Option<ProviderInvoice>,
    ) -> Statement {
        Statement::from(StatementParts {
            id: statement.id(),
            organisation_id: statement.organisation_id(),
            period: statement.period(),
            lines: statement.lines().to_vec(),
            status,
            invoice,
            created_at: statement.created_at(),
            updated_at: at(1),
        })
    }

    fn invoice() -> ProviderInvoice {
        ProviderInvoice {
            id: ProviderInvoiceId::parse("in_123").unwrap(),
            hosted_url: "https://pay.test/in_123".to_string(),
        }
    }

    fn statements_holding(stored: Option<Statement>) -> MockStatementRepository {
        let mut mock = MockStatementRepository::new();
        mock.expect_find().returning(move |id| {
            let stored = stored.clone().filter(|stored| stored.id() == *id);
            Box::pin(async move { Ok(stored) })
        });
        mock
    }

    fn statements_holding_and_noting(
        stored: Option<Statement>,
        updates: &Updates,
    ) -> MockStatementRepository {
        let mut mock = statements_holding(stored);
        let updates = updates.clone();
        mock.expect_update().returning(move |statement| {
            updates.note(statement);
            Box::pin(async { Ok(()) })
        });
        mock
    }

    fn strict_sending() -> Sending {
        StatementSending::new(
            MockBillingAccountRepository::new(),
            MockStatementRepository::new(),
            MockPaymentProvider::new(),
        )
    }

    fn accounts_holding(account: Option<BillingAccount>) -> MockBillingAccountRepository {
        let mut mock = MockBillingAccountRepository::new();
        mock.expect_find().returning(move |_| {
            let account = account.clone();
            Box::pin(async move { Ok(account) })
        });
        mock
    }

    fn statements_noting(updates: &Updates) -> MockStatementRepository {
        let mut mock = MockStatementRepository::new();
        let updates = updates.clone();
        mock.expect_update().returning(move |statement| {
            updates.note(statement);
            Box::pin(async { Ok(()) })
        });
        mock
    }

    fn provider_issuing(
        issued: &Arc<Mutex<Vec<(ProviderCustomerId, StatementStatus)>>>,
    ) -> MockPaymentProvider {
        let mut mock = MockPaymentProvider::new();
        let issued = Arc::clone(issued);
        mock.expect_issue_invoice()
            .returning(move |customer, statement| {
                issued
                    .lock()
                    .expect("not poisoned")
                    .push((customer.clone(), statement.status()));
                Box::pin(async { Ok(invoice()) })
            });
        mock
    }

    #[tokio::test]
    async fn a_held_statement_whose_account_can_now_be_invoiced_is_invoiced_and_sent() {
        let updates = Updates::default();
        let issued = Arc::new(Mutex::new(Vec::new()));
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Valid))),
            statements_noting(&updates),
            provider_issuing(&issued),
        );

        let sent = sending
            .send(statement(StatementStatus::Held), at(2))
            .await
            .expect("sent");

        assert_eq!(sent.status(), StatementStatus::Sent);
        assert_eq!(sent.invoice(), Some(&invoice()));
        assert_eq!(sent.updated_at(), at(2));
        assert_eq!(
            updates.statuses(),
            vec![StatementStatus::Draft, StatementStatus::Sent]
        );
        assert_eq!(
            *issued.lock().expect("not poisoned"),
            vec![(
                ProviderCustomerId::parse("cus_123").unwrap(),
                StatementStatus::Draft
            )]
        );
    }

    #[tokio::test]
    async fn a_held_statement_whose_account_still_cannot_be_invoiced_is_unchanged() {
        let held = statement(StatementStatus::Held);

        for stored in [
            None,
            Some(account(false, PaymentMethodStatus::Valid)),
            Some(account(true, PaymentMethodStatus::Missing)),
        ] {
            let sending = StatementSending::new(
                accounts_holding(stored.clone()),
                MockStatementRepository::new(),
                MockPaymentProvider::new(),
            );

            let sent = sending.send(held.clone(), at(2)).await.expect("left alone");

            assert_eq!(sent, held, "{stored:?}");
        }
    }

    #[tokio::test]
    async fn a_draft_statement_is_sent_with_the_invoice_of_the_provider() {
        let updates = Updates::default();
        let issued = Arc::new(Mutex::new(Vec::new()));
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Failing))),
            statements_noting(&updates),
            provider_issuing(&issued),
        );
        let draft = statement(StatementStatus::Draft);

        let sent = sending.send(draft.clone(), at(2)).await.expect("sent");

        assert_eq!(sent.status(), StatementStatus::Sent);
        assert_eq!(sent.invoice(), Some(&invoice()));
        assert_eq!(sent.id(), draft.id());
        assert_eq!(sent.lines(), draft.lines());
        assert_eq!(updates.statuses(), vec![StatementStatus::Sent]);
        assert_eq!(issued.lock().expect("not poisoned").len(), 1);
    }

    #[tokio::test]
    async fn the_stored_update_carries_the_invoice() {
        let updates = Updates::default();
        let issued = Arc::new(Mutex::new(Vec::new()));
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Valid))),
            statements_noting(&updates),
            provider_issuing(&issued),
        );

        sending
            .send(statement(StatementStatus::Draft), at(2))
            .await
            .expect("sent");

        assert_eq!(
            updates.0.lock().expect("not poisoned").clone(),
            vec![(StatementStatus::Sent, Some(invoice()))]
        );
    }

    #[tokio::test]
    async fn a_draft_statement_without_a_customer_is_unchanged() {
        let sending = StatementSending::new(
            accounts_holding(Some(account(false, PaymentMethodStatus::Valid))),
            MockStatementRepository::new(),
            MockPaymentProvider::new(),
        );
        let draft = statement(StatementStatus::Draft);

        let sent = sending
            .send(draft.clone(), at(2))
            .await
            .expect("left alone");

        assert_eq!(sent, draft);
    }

    #[tokio::test]
    async fn every_other_statement_is_left_alone_and_nothing_is_asked() {
        for status in [
            StatementStatus::Zero,
            StatementStatus::Sent,
            StatementStatus::Paid,
            StatementStatus::PaymentFailed,
            StatementStatus::Uncollectible,
        ] {
            let stored = statement(status);

            let sent = strict_sending()
                .send(stored.clone(), at(2))
                .await
                .expect("left alone");

            assert_eq!(sent, stored, "{status}");
        }
    }

    #[tokio::test]
    async fn a_provider_error_leaves_a_draft_statement_a_draft_and_is_returned() {
        let updates = Updates::default();
        let mut provider = MockPaymentProvider::new();
        provider.expect_issue_invoice().times(1).returning(|_, _| {
            Box::pin(async {
                Err(BillingError::Provider {
                    reason: "unreachable".to_string(),
                }
                .into())
            })
        });
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Valid))),
            statements_noting(&updates),
            provider,
        );

        let failed = sending
            .send(statement(StatementStatus::Draft), at(2))
            .await
            .expect_err("the provider failed");

        assert!(matches!(
            failed,
            CoreError::Billing(BillingError::Provider { .. })
        ));
        assert!(updates.statuses().is_empty());
    }

    #[tokio::test]
    async fn a_provider_error_after_a_held_statement_became_a_draft_stores_the_draft_only() {
        let updates = Updates::default();
        let mut provider = MockPaymentProvider::new();
        provider.expect_issue_invoice().times(1).returning(|_, _| {
            Box::pin(async {
                Err(BillingError::Provider {
                    reason: "unreachable".to_string(),
                }
                .into())
            })
        });
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Valid))),
            statements_noting(&updates),
            provider,
        );

        let failed = sending
            .send(statement(StatementStatus::Held), at(2))
            .await
            .expect_err("the provider failed");

        assert!(matches!(
            failed,
            CoreError::Billing(BillingError::Provider { .. })
        ));
        assert_eq!(updates.statuses(), vec![StatementStatus::Draft]);
    }

    #[tokio::test]
    async fn the_stored_copy_is_what_gets_sent_not_the_one_read_earlier() {
        let updates = Updates::default();
        let issued = Arc::new(Mutex::new(Vec::new()));
        let stale = statement(StatementStatus::Held);
        let stored = moved_to(&stale, StatementStatus::Draft, None);
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Valid))),
            statements_holding_and_noting(Some(stored), &updates),
            provider_issuing(&issued),
        );

        let sent = sending.send_current(stale, at(2)).await.expect("sent");

        assert_eq!(sent.status(), StatementStatus::Sent);
        assert_eq!(updates.statuses(), vec![StatementStatus::Sent]);
        assert_eq!(
            *issued.lock().expect("not poisoned"),
            vec![(
                ProviderCustomerId::parse("cus_123").unwrap(),
                StatementStatus::Draft
            )]
        );
    }

    #[tokio::test]
    async fn a_statement_that_moved_since_it_was_read_is_returned_unchanged_and_nothing_is_asked() {
        for moved in [
            StatementStatus::Zero,
            StatementStatus::Sent,
            StatementStatus::Paid,
            StatementStatus::PaymentFailed,
            StatementStatus::Uncollectible,
        ] {
            let stale = statement(StatementStatus::Draft);
            let stored = moved_to(&stale, moved, Some(invoice()));
            let sending = StatementSending::new(
                MockBillingAccountRepository::new(),
                statements_holding(Some(stored)),
                MockPaymentProvider::new(),
            );

            let left = sending
                .send_current(stale.clone(), at(2))
                .await
                .expect("left alone");

            assert_eq!(left, stale, "{moved}");
        }
    }

    #[tokio::test]
    async fn a_statement_that_is_gone_is_returned_unchanged_and_nothing_is_asked() {
        let stale = statement(StatementStatus::Held);
        let sending = StatementSending::new(
            MockBillingAccountRepository::new(),
            statements_holding(None),
            MockPaymentProvider::new(),
        );

        let left = sending
            .send_current(stale.clone(), at(2))
            .await
            .expect("left alone");

        assert_eq!(left, stale);
    }

    #[tokio::test]
    async fn the_stored_copy_that_still_cannot_be_invoiced_is_left_as_it_is() {
        let held = statement(StatementStatus::Held);
        let sending = StatementSending::new(
            accounts_holding(Some(account(true, PaymentMethodStatus::Missing))),
            statements_holding(Some(held.clone())),
            MockPaymentProvider::new(),
        );

        let left = sending
            .send_current(held.clone(), at(2))
            .await
            .expect("left alone");

        assert_eq!(left, held);
    }
}
