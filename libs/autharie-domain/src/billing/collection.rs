use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    billing::{
        BillingError, BillingSuspension, BillingSuspensionId, PaymentMethodStatus,
        ProviderCustomerId, ProviderInvoiceId, ProviderPaymentMethodId, Statement, StatementId,
        StatementStatus, Subscription,
        ports::{
            BillingAccountRepository, BillingSuspensionRepository, DeploymentPower,
            PaymentProvider, StatementRepository, SubscriptionRepository,
        },
    },
    deployments::DeploymentId,
    organisation::{OrganisationId, ports::OrganisationRepository},
};

pub struct Collection<ST, A, SU, S, O, D> {
    pub statements: ST,
    pub accounts: A,
    pub suspensions: SU,
    pub subscriptions: S,
    pub organisations: O,
    pub power: D,
}

impl<ST, A, SU, S, O, D> Collection<ST, A, SU, S, O, D>
where
    ST: StatementRepository,
    A: BillingAccountRepository,
    SU: BillingSuspensionRepository,
    S: SubscriptionRepository,
    O: OrganisationRepository,
    D: DeploymentPower,
{
    pub async fn invoice_paid(
        &self,
        invoice: &ProviderInvoiceId,
        statement: Option<StatementId>,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let Some(mut statement) = self.statement_of(invoice, statement).await? else {
            return Ok(());
        };
        let organisation_id = statement.organisation_id();

        self.advance(&mut statement, StatementStatus::Paid, now)
            .await?;
        self.mark_payment_method(&organisation_id, PaymentMethodStatus::Valid)
            .await?;
        self.resume(&organisation_id, now).await
    }

    pub async fn payment_failed(
        &self,
        invoice: &ProviderInvoiceId,
        statement: Option<StatementId>,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let Some(mut statement) = self.statement_of(invoice, statement).await? else {
            return Ok(());
        };
        let organisation_id = statement.organisation_id();

        self.advance(&mut statement, StatementStatus::PaymentFailed, now)
            .await?;

        if statement.status() != StatementStatus::PaymentFailed {
            return Ok(());
        }

        self.mark_payment_method(&organisation_id, PaymentMethodStatus::Failing)
            .await
    }

    pub async fn uncollectible(
        &self,
        invoice: &ProviderInvoiceId,
        statement: Option<StatementId>,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let Some(mut statement) = self.statement_of(invoice, statement).await? else {
            return Ok(());
        };

        self.advance(&mut statement, StatementStatus::Uncollectible, now)
            .await?;

        if statement.status() != StatementStatus::Uncollectible {
            return Ok(());
        }

        let organisation_id = statement.organisation_id();

        if self
            .suspensions
            .find_active(&organisation_id)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let suspended_organisation = self.suspend_organisation(&organisation_id).await?;
        let stopped = self.stop_billable(&organisation_id, now).await?;

        self.suspensions
            .insert(&BillingSuspension {
                id: BillingSuspensionId::new(),
                organisation_id,
                statement_id: statement.id(),
                stopped,
                suspended_organisation,
                suspended_at: now,
                lifted_at: None,
            })
            .await
    }

    /// Stops what billing could not stop when it suspended the organisation
    /// (a deployment still coming up, or upgrading), now that it can.
    ///
    /// Recorded on the suspension, so payment starts these too.
    pub async fn keep_suspended(
        &self,
        organisation_id: &OrganisationId,
        now: DateTime<Utc>,
    ) -> Result<Vec<DeploymentId>, CoreError> {
        let Some(suspension) = self.suspensions.find_active(organisation_id).await? else {
            return Ok(Vec::new());
        };

        let stopped = self.stop_billable(organisation_id, now).await?;

        if !stopped.is_empty() {
            self.suspensions
                .add_stopped(&suspension.id, &stopped)
                .await?;
        }

        Ok(stopped)
    }

    async fn stop_billable(
        &self,
        organisation_id: &OrganisationId,
        now: DateTime<Utc>,
    ) -> Result<Vec<DeploymentId>, CoreError> {
        let stopped = self.power.stop_billable(organisation_id).await?;

        for deployment_id in &stopped {
            self.close_open_subscription(deployment_id, now).await?;
        }

        Ok(stopped)
    }

    async fn statement_of(
        &self,
        invoice: &ProviderInvoiceId,
        named: Option<StatementId>,
    ) -> Result<Option<Statement>, CoreError> {
        if let Some(statement) = self.statements.find_by_invoice(invoice).await? {
            return Ok(Some(statement));
        }

        let Some(named) = named else {
            return Ok(None);
        };

        if self.statements.find(&named).await?.is_none() {
            return Ok(None);
        }

        Err(BillingError::InvoiceNotRecordedYet {
            invoice: invoice.as_str().to_string(),
        }
        .into())
    }

    async fn advance(
        &self,
        statement: &mut Statement,
        next: StatementStatus,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        if statement.status().can_become(next) {
            statement.advance(next, now)?;
            self.statements.update(statement).await?;
        }

        Ok(())
    }

    async fn mark_payment_method(
        &self,
        organisation_id: &OrganisationId,
        payment_method: PaymentMethodStatus,
    ) -> Result<(), CoreError> {
        let Some(mut account) = self.accounts.find(organisation_id).await? else {
            return Ok(());
        };

        account.payment_method = payment_method;
        self.accounts.save(&account).await
    }

    async fn resume(
        &self,
        organisation_id: &OrganisationId,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let Some(suspension) = self.suspensions.find_active(organisation_id).await? else {
            return Ok(());
        };

        let still_owing = self
            .statements
            .list_for(organisation_id)
            .await?
            .iter()
            .any(|statement| statement.status() == StatementStatus::Uncollectible);

        if still_owing {
            return Ok(());
        }

        let started = self.power.start(&suspension.stopped).await?;

        for deployment_id in &started {
            self.reopen_subscription(deployment_id, now).await?;
        }

        self.suspensions.lift(&suspension.id, now).await?;

        if suspension.suspended_organisation {
            self.activate_organisation(organisation_id).await?;
        }

        Ok(())
    }

    async fn suspend_organisation(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<bool, CoreError> {
        match self.organisations.find_by_id(organisation_id).await? {
            Some(mut organisation)
                if !organisation.is_suspended() && !organisation.is_deleted() =>
            {
                organisation.suspend()?;
                self.organisations.update(organisation).await?;

                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn activate_organisation(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<(), CoreError> {
        match self.organisations.find_by_id(organisation_id).await? {
            Some(mut organisation) if organisation.is_suspended() => {
                organisation.activate()?;
                self.organisations.update(organisation).await?;

                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn close_open_subscription(
        &self,
        deployment_id: &DeploymentId,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let Some(open) = self
            .subscriptions
            .latest_for(deployment_id)
            .await?
            .filter(Subscription::is_open)
        else {
            return Ok(());
        };

        let closed = open.close(now)?;
        self.subscriptions.close(&closed).await
    }

    async fn reopen_subscription(
        &self,
        deployment_id: &DeploymentId,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let Some(closed) = self
            .subscriptions
            .latest_for(deployment_id)
            .await?
            .filter(|subscription| !subscription.is_open())
        else {
            return Ok(());
        };

        let next = closed.successor(closed.terms().clone(), now);
        self.subscriptions.open(&next).await
    }
}

pub async fn adopt_payment_method<A: BillingAccountRepository, P: PaymentProvider>(
    accounts: &A,
    provider: &P,
    customer: &ProviderCustomerId,
    payment_method: &ProviderPaymentMethodId,
) -> Result<(), CoreError> {
    let Some(mut account) = accounts.find_by_customer(customer).await? else {
        return Ok(());
    };

    provider
        .set_default_payment_method(customer, payment_method)
        .await?;

    account.payment_method = PaymentMethodStatus::Valid;
    accounts.save(&account).await
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{Arc, Mutex},
    };

    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;
    use crate::{
        billing::{
            BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
            BillingError, BillingName, BillingPeriod, Money, ProviderInvoice, StatementId,
            StatementLine, StatementParts, SubscriptionId, SubscriptionParts, SubscriptionTerms,
            TierCode,
            ports::{
                MockBillingAccountRepository, MockBillingSuspensionRepository, MockPaymentProvider,
                MockStatementRepository, MockSubscriptionRepository,
            },
        },
        offers::Offer,
        organisation::{
            Organisation,
            ports::MockOrganisationRepository,
            value_objects::{OrganisationName, OrganisationSlug, OrganisationStatus, Plan},
        },
        user::UserId,
    };

    #[derive(Debug, Clone, PartialEq)]
    enum Written {
        Statement(StatementId, StatementStatus),
        Account(PaymentMethodStatus),
        Organisation(OrganisationStatus),
        Stopped(OrganisationId),
        Started(Vec<DeploymentId>),
        Closed(DeploymentId, Option<DateTime<Utc>>),
        Opened(DeploymentId, SubscriptionTerms, DateTime<Utc>),
        Suspended {
            statement: StatementId,
            stopped: Vec<DeploymentId>,
            organisation: bool,
            at: DateTime<Utc>,
            lifted: Option<DateTime<Utc>>,
        },
        Extended(BillingSuspensionId, Vec<DeploymentId>),
        Lifted(BillingSuspensionId, DateTime<Utc>),
    }

    #[derive(Default)]
    struct World {
        statements: Vec<Statement>,
        account: Option<BillingAccount>,
        suspensions: Vec<BillingSuspension>,
        subscriptions: Vec<Subscription>,
        organisation: Option<Organisation>,
        stoppable: Vec<DeploymentId>,
        restartable: Vec<DeploymentId>,
        written: Vec<Written>,
    }

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<World>>);

    impl Shared {
        fn with<R>(&self, act: impl FnOnce(&mut World) -> R) -> R {
            act(&mut self.0.lock().expect("not poisoned"))
        }
    }

    struct FakePower(Shared);

    impl DeploymentPower for FakePower {
        async fn stop_billable(
            &self,
            organisation_id: &OrganisationId,
        ) -> Result<Vec<DeploymentId>, CoreError> {
            Ok(self.0.with(|world| {
                world.written.push(Written::Stopped(*organisation_id));
                world.stoppable.clone()
            }))
        }

        async fn start(
            &self,
            deployment_ids: &[DeploymentId],
        ) -> Result<Vec<DeploymentId>, CoreError> {
            Ok(self.0.with(|world| {
                world
                    .written
                    .push(Written::Started(deployment_ids.to_vec()));
                deployment_ids
                    .iter()
                    .filter(|id| world.restartable.contains(id))
                    .copied()
                    .collect()
            }))
        }
    }

    type Subject = Collection<
        MockStatementRepository,
        MockBillingAccountRepository,
        MockBillingSuspensionRepository,
        MockSubscriptionRepository,
        MockOrganisationRepository,
        FakePower,
    >;

    struct Fixture {
        world: Shared,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                world: Shared::default(),
            }
        }

        fn with_statement(self, statement: Statement) -> Self {
            self.world.with(|world| world.statements.push(statement));
            self
        }

        fn with_account(self, account: BillingAccount) -> Self {
            self.world.with(|world| world.account = Some(account));
            self
        }

        fn with_suspension(self, suspension: BillingSuspension) -> Self {
            self.world.with(|world| world.suspensions.push(suspension));
            self
        }

        fn with_subscription(self, subscription: Subscription) -> Self {
            self.world
                .with(|world| world.subscriptions.push(subscription));
            self
        }

        fn with_organisation(self, organisation: Organisation) -> Self {
            self.world
                .with(|world| world.organisation = Some(organisation));
            self
        }

        fn stopping(self, ids: &[DeploymentId]) -> Self {
            self.world.with(|world| world.stoppable = ids.to_vec());
            self
        }

        fn restarting(self, ids: &[DeploymentId]) -> Self {
            self.world.with(|world| world.restartable = ids.to_vec());
            self
        }

        fn written(&self) -> Vec<Written> {
            self.world.with(|world| world.written.clone())
        }

        fn statement(&self, id: StatementId) -> Statement {
            self.world.with(|world| {
                world
                    .statements
                    .iter()
                    .find(|statement| statement.id() == id)
                    .cloned()
                    .expect("a stored statement")
            })
        }

        fn collection(&self) -> Subject {
            Collection {
                statements: self.statements(),
                accounts: self.accounts(),
                suspensions: self.suspensions(),
                subscriptions: self.subscriptions(),
                organisations: self.organisations(),
                power: FakePower(self.world.clone()),
            }
        }

        fn statements(&self) -> MockStatementRepository {
            let mut mock = MockStatementRepository::new();
            let world = self.world.clone();
            mock.expect_find_by_invoice().returning(move |invoice| {
                let found = world.with(|world| {
                    world
                        .statements
                        .iter()
                        .find(|statement| {
                            statement.invoice().map(|issued| &issued.id) == Some(invoice)
                        })
                        .cloned()
                });
                Box::pin(async move { Ok(found) })
            });
            let world = self.world.clone();
            mock.expect_find().returning(move |id| {
                let found = world.with(|world| {
                    world
                        .statements
                        .iter()
                        .find(|statement| statement.id() == *id)
                        .cloned()
                });
                Box::pin(async move { Ok(found) })
            });
            let world = self.world.clone();
            mock.expect_list_for().returning(move |organisation_id| {
                let listed = world.with(|world| {
                    world
                        .statements
                        .iter()
                        .filter(|statement| statement.organisation_id() == *organisation_id)
                        .cloned()
                        .collect::<Vec<_>>()
                });
                Box::pin(async move { Ok(listed) })
            });
            let world = self.world.clone();
            mock.expect_update().returning(move |updated| {
                world.with(|world| {
                    if let Some(stored) = world
                        .statements
                        .iter_mut()
                        .find(|statement| statement.id() == updated.id())
                    {
                        *stored = updated.clone();
                    }
                    world
                        .written
                        .push(Written::Statement(updated.id(), updated.status()));
                });
                Box::pin(async { Ok(()) })
            });
            mock
        }

        fn accounts(&self) -> MockBillingAccountRepository {
            let mut mock = MockBillingAccountRepository::new();
            let world = self.world.clone();
            mock.expect_find().returning(move |_| {
                let found = world.with(|world| world.account.clone());
                Box::pin(async move { Ok(found) })
            });
            let world = self.world.clone();
            mock.expect_save().returning(move |account| {
                world.with(|world| {
                    world.account = Some(account.clone());
                    world.written.push(Written::Account(account.payment_method));
                });
                Box::pin(async { Ok(()) })
            });
            mock
        }

        fn suspensions(&self) -> MockBillingSuspensionRepository {
            let mut mock = MockBillingSuspensionRepository::new();
            let world = self.world.clone();
            mock.expect_find_active().returning(move |organisation_id| {
                let found = world.with(|world| {
                    world
                        .suspensions
                        .iter()
                        .find(|suspension| {
                            suspension.organisation_id == *organisation_id && suspension.is_active()
                        })
                        .cloned()
                });
                Box::pin(async move { Ok(found) })
            });
            let world = self.world.clone();
            mock.expect_insert().returning(move |suspension| {
                world.with(|world| {
                    world.suspensions.push(suspension.clone());
                    world.written.push(Written::Suspended {
                        statement: suspension.statement_id,
                        stopped: suspension.stopped.clone(),
                        organisation: suspension.suspended_organisation,
                        at: suspension.suspended_at,
                        lifted: suspension.lifted_at,
                    });
                });
                Box::pin(async { Ok(()) })
            });
            let world = self.world.clone();
            mock.expect_add_stopped().returning(move |id, deployments| {
                world.with(|world| {
                    if let Some(stored) = world
                        .suspensions
                        .iter_mut()
                        .find(|suspension| suspension.id == *id)
                    {
                        stored.stopped.extend_from_slice(deployments);
                    }
                    world
                        .written
                        .push(Written::Extended(*id, deployments.to_vec()));
                });
                Box::pin(async { Ok(()) })
            });
            let world = self.world.clone();
            mock.expect_lift().returning(move |id, at| {
                world.with(|world| {
                    if let Some(stored) = world
                        .suspensions
                        .iter_mut()
                        .find(|suspension| suspension.id == *id)
                    {
                        stored.lifted_at = Some(at);
                    }
                    world.written.push(Written::Lifted(*id, at));
                });
                Box::pin(async { Ok(()) })
            });
            mock
        }

        fn subscriptions(&self) -> MockSubscriptionRepository {
            let mut mock = MockSubscriptionRepository::new();
            let world = self.world.clone();
            mock.expect_latest_for().returning(move |deployment_id| {
                let found = world.with(|world| {
                    world
                        .subscriptions
                        .iter()
                        .rev()
                        .find(|subscription| subscription.deployment_id() == *deployment_id)
                        .cloned()
                });
                Box::pin(async move { Ok(found) })
            });
            let world = self.world.clone();
            mock.expect_open().returning(move |subscription| {
                world.with(|world| {
                    world.subscriptions.push(subscription.clone());
                    world.written.push(Written::Opened(
                        subscription.deployment_id(),
                        subscription.terms().clone(),
                        subscription.starts_at(),
                    ));
                });
                Box::pin(async { Ok(()) })
            });
            let world = self.world.clone();
            mock.expect_close().returning(move |closed| {
                world.with(|world| {
                    if let Some(stored) = world
                        .subscriptions
                        .iter_mut()
                        .find(|subscription| subscription.id() == closed.id())
                    {
                        *stored = closed.clone();
                    }
                    world
                        .written
                        .push(Written::Closed(closed.deployment_id(), closed.ends_at()));
                });
                Box::pin(async { Ok(()) })
            });
            mock
        }

        fn organisations(&self) -> MockOrganisationRepository {
            let mut mock = MockOrganisationRepository::new();
            let world = self.world.clone();
            mock.expect_find_by_id().returning(move |_| {
                let found = world.with(|world| world.organisation.clone());
                Box::pin(async move { Ok(found) })
            });
            let world = self.world.clone();
            mock.expect_update().returning(move |organisation| {
                world.with(|world| {
                    world.organisation = Some(organisation.clone());
                    world
                        .written
                        .push(Written::Organisation(organisation.status));
                });
                Box::pin(async move { Ok(organisation) })
            });
            mock
        }
    }

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, day, 10, 0, 0).unwrap()
    }

    fn period() -> BillingPeriod {
        BillingPeriod::new(2026, 11).unwrap()
    }

    fn deployment(n: u128) -> DeploymentId {
        DeploymentId(Uuid::from_u128(n))
    }

    fn invoice_id(raw: &str) -> ProviderInvoiceId {
        ProviderInvoiceId::parse(raw).unwrap()
    }

    fn organisation() -> Organisation {
        Organisation::new(
            OrganisationName::new("FerrisLabs").expect("a name"),
            OrganisationSlug::new("ferrislabs").expect("a slug"),
            UserId(Uuid::from_u128(7)),
            Plan::Free,
        )
    }

    fn suspended_organisation() -> Organisation {
        let mut organisation = organisation();
        organisation.suspend().expect("suspendable");
        organisation
    }

    fn parts(
        organisation_id: OrganisationId,
        invoice: Option<ProviderInvoice>,
        status: StatementStatus,
    ) -> StatementParts {
        StatementParts {
            id: StatementId::new(),
            organisation_id,
            period: period(),
            lines: vec![StatementLine {
                deployment_id: deployment(1),
                subscription_id: SubscriptionId::new(),
                tier: TierCode::legacy(Offer::Standard),
                monthly_price: Money::from_minor(4900),
                starts_at: period().starts_at(),
                ends_at: period().ends_at(),
                covered_seconds: period().seconds(),
                amount: Money::from_minor(4900),
            }],
            status,
            invoice,
            created_at: at(1),
            updated_at: at(1),
        }
    }

    fn statement(
        organisation_id: OrganisationId,
        invoice: &str,
        status: StatementStatus,
    ) -> Statement {
        Statement::from(parts(
            organisation_id,
            Some(ProviderInvoice {
                id: invoice_id(invoice),
                hosted_url: format!("https://pay.test/{invoice}"),
            }),
            status,
        ))
    }

    fn unrecorded(organisation_id: OrganisationId) -> Statement {
        Statement::from(parts(organisation_id, None, StatementStatus::Draft))
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

    fn account(
        organisation_id: OrganisationId,
        payment_method: PaymentMethodStatus,
    ) -> BillingAccount {
        BillingAccount {
            customer: Some(ProviderCustomerId::parse("cus_123").unwrap()),
            payment_method,
            ..BillingAccount::new(organisation_id, details())
        }
    }

    fn terms(price: i64) -> SubscriptionTerms {
        SubscriptionTerms {
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(price),
            offer: Offer::Standard,
            features: BTreeSet::new(),
        }
    }

    fn subscription(
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        price: i64,
        ends_at: Option<DateTime<Utc>>,
    ) -> Subscription {
        Subscription::try_from(SubscriptionParts {
            id: SubscriptionId::new(),
            organisation_id,
            deployment_id,
            terms: terms(price),
            starts_at: at(1),
            ends_at,
        })
        .unwrap()
    }

    fn open_subscription(
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        price: i64,
    ) -> Subscription {
        subscription(organisation_id, deployment_id, price, None)
    }

    fn closed_subscription(
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        price: i64,
    ) -> Subscription {
        subscription(organisation_id, deployment_id, price, Some(at(10)))
    }

    fn suspension(
        organisation_id: OrganisationId,
        statement_id: StatementId,
        stopped: &[DeploymentId],
        suspended_organisation: bool,
    ) -> BillingSuspension {
        BillingSuspension {
            id: BillingSuspensionId::new(),
            organisation_id,
            statement_id,
            stopped: stopped.to_vec(),
            suspended_organisation,
            suspended_at: at(10),
            lifted_at: None,
        }
    }

    #[tokio::test]
    async fn paid_marks_the_statement_paid_and_the_payment_method_valid() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(sent.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Failing));

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(sent.id(), StatementStatus::Paid),
                Written::Account(PaymentMethodStatus::Valid),
            ]
        );
        assert_eq!(fixture.statement(sent.id()).updated_at(), at(21));
    }

    #[tokio::test]
    async fn paid_without_an_account_marks_only_the_statement() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new().with_statement(sent.clone());

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![Written::Statement(sent.id(), StatementStatus::Paid)]
        );
    }

    #[tokio::test]
    async fn a_replayed_paid_event_rewrites_no_statement() {
        let organisation = organisation();
        let paid = statement(organisation.id, "in_1", StatementStatus::Paid);
        let fixture = Fixture::new()
            .with_statement(paid)
            .with_account(account(organisation.id, PaymentMethodStatus::Valid));

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![Written::Account(PaymentMethodStatus::Valid)]
        );
    }

    #[tokio::test]
    async fn paying_the_statement_billing_suspended_over_restarts_reopens_lifts_and_activates() {
        let organisation = suspended_organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let stopped = [deployment(1), deployment(2)];
        let held = suspension(organisation.id, uncollectible.id(), &stopped, true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(held.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Failing))
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(closed_subscription(organisation.id, deployment(2), 9900))
            .restarting(&stopped);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(uncollectible.id(), StatementStatus::Paid),
                Written::Account(PaymentMethodStatus::Valid),
                Written::Started(stopped.to_vec()),
                Written::Opened(deployment(1), terms(4900), at(21)),
                Written::Opened(deployment(2), terms(9900), at(21)),
                Written::Lifted(held.id, at(21)),
                Written::Organisation(OrganisationStatus::Active),
            ]
        );
    }

    #[tokio::test]
    async fn a_deployment_that_could_not_be_stopped_at_first_is_stopped_closed_and_recorded() {
        let organisation = organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let held = suspension(organisation.id, uncollectible.id(), &[deployment(1)], true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible)
            .with_suspension(held.clone())
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(open_subscription(organisation.id, deployment(2), 9900))
            .stopping(&[deployment(2)]);

        let stopped = fixture
            .collection()
            .keep_suspended(&organisation.id, at(25))
            .await
            .expect("kept");

        assert_eq!(stopped, vec![deployment(2)]);
        assert_eq!(
            fixture.written(),
            vec![
                Written::Stopped(organisation.id),
                Written::Closed(deployment(2), Some(at(25))),
                Written::Extended(held.id, vec![deployment(2)]),
            ]
        );
        assert_eq!(
            fixture
                .world
                .with(|world| world.suspensions[0].stopped.clone()),
            vec![deployment(1), deployment(2)]
        );
    }

    #[tokio::test]
    async fn keeping_a_suspension_with_nothing_left_to_stop_writes_nothing() {
        let organisation = organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(suspension(
                organisation.id,
                uncollectible.id(),
                &[deployment(1)],
                true,
            ))
            .stopping(&[]);

        let stopped = fixture
            .collection()
            .keep_suspended(&organisation.id, at(25))
            .await
            .expect("kept");

        assert!(stopped.is_empty());
        assert_eq!(fixture.written(), vec![Written::Stopped(organisation.id)]);
    }

    #[tokio::test]
    async fn an_organisation_with_no_active_suspension_is_not_touched() {
        let organisation = organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Paid);
        let lifted = BillingSuspension {
            lifted_at: Some(at(22)),
            ..suspension(organisation.id, uncollectible.id(), &[deployment(1)], true)
        };
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible)
            .with_suspension(lifted)
            .with_subscription(open_subscription(organisation.id, deployment(2), 9900))
            .stopping(&[deployment(2)]);

        let stopped = fixture
            .collection()
            .keep_suspended(&organisation.id, at(25))
            .await
            .expect("kept");

        assert!(stopped.is_empty());
        assert!(fixture.written().is_empty());
    }

    #[tokio::test]
    async fn paying_starts_the_deployments_stopped_later_as_well() {
        let organisation = suspended_organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let held = suspension(organisation.id, uncollectible.id(), &[deployment(1)], true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(held.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Failing))
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(open_subscription(organisation.id, deployment(2), 9900))
            .stopping(&[deployment(2)])
            .restarting(&[deployment(1), deployment(2)]);
        let collection = fixture.collection();
        collection
            .keep_suspended(&organisation.id, at(25))
            .await
            .expect("kept");

        collection
            .invoice_paid(&invoice_id("in_1"), None, at(26))
            .await
            .expect("collected");

        let written = fixture.written();
        assert!(
            written.contains(&Written::Started(vec![deployment(1), deployment(2)])),
            "{written:?}"
        );
        assert!(written.contains(&Written::Opened(deployment(2), terms(9900), at(26))));
        assert!(written.contains(&Written::Lifted(held.id, at(26))));
    }

    #[tokio::test]
    async fn a_reopened_subscription_is_a_new_open_one_of_the_same_deployment_and_terms() {
        let organisation = suspended_organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let closed = closed_subscription(organisation.id, deployment(1), 4900);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(suspension(
                organisation.id,
                uncollectible.id(),
                &[deployment(1)],
                true,
            ))
            .with_subscription(closed.clone())
            .restarting(&[deployment(1)]);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        let latest = fixture.world.with(|world| {
            world
                .subscriptions
                .iter()
                .rev()
                .find(|subscription| subscription.deployment_id() == deployment(1))
                .cloned()
                .expect("a subscription")
        });
        assert_ne!(latest.id(), closed.id());
        assert!(latest.is_open());
        assert_eq!(latest.organisation_id(), organisation.id);
        assert_eq!(latest.terms(), closed.terms());
        assert_eq!(latest.starts_at(), at(21));
    }

    #[tokio::test]
    async fn when_only_some_deployments_restart_only_those_are_reopened() {
        let organisation = suspended_organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let stopped = [deployment(1), deployment(2)];
        let held = suspension(organisation.id, uncollectible.id(), &stopped, true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(held.clone())
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(closed_subscription(organisation.id, deployment(2), 9900))
            .restarting(&[deployment(2)]);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(uncollectible.id(), StatementStatus::Paid),
                Written::Started(stopped.to_vec()),
                Written::Opened(deployment(2), terms(9900), at(21)),
                Written::Lifted(held.id, at(21)),
                Written::Organisation(OrganisationStatus::Active),
            ]
        );
    }

    #[tokio::test]
    async fn only_a_closed_latest_subscription_is_reopened() {
        let organisation = suspended_organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let stopped = [deployment(1), deployment(2), deployment(3)];
        let held = suspension(organisation.id, uncollectible.id(), &stopped, true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(held.clone())
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(closed_subscription(organisation.id, deployment(2), 9900))
            .with_subscription(open_subscription(organisation.id, deployment(2), 9900))
            .restarting(&stopped);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(uncollectible.id(), StatementStatus::Paid),
                Written::Started(stopped.to_vec()),
                Written::Opened(deployment(1), terms(4900), at(21)),
                Written::Lifted(held.id, at(21)),
                Written::Organisation(OrganisationStatus::Active),
            ]
        );
    }

    #[tokio::test]
    async fn paying_while_another_statement_is_still_uncollectible_keeps_the_suspension() {
        let organisation = suspended_organisation();
        let earlier = statement(organisation.id, "in_0", StatementStatus::Uncollectible);
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let held = suspension(organisation.id, earlier.id(), &[deployment(1)], true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(earlier)
            .with_statement(sent.clone())
            .with_suspension(held)
            .with_account(account(organisation.id, PaymentMethodStatus::Failing))
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .restarting(&[deployment(1)]);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(sent.id(), StatementStatus::Paid),
                Written::Account(PaymentMethodStatus::Valid),
            ]
        );
        assert!(
            fixture
                .world
                .with(|world| world.suspensions.iter().all(BillingSuspension::is_active))
        );
    }

    #[tokio::test]
    async fn lifting_a_suspension_billing_did_not_make_leaves_the_organisation_suspended() {
        let organisation = suspended_organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let held = suspension(organisation.id, uncollectible.id(), &[deployment(1)], false);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(held.clone())
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .restarting(&[deployment(1)]);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(uncollectible.id(), StatementStatus::Paid),
                Written::Started(vec![deployment(1)]),
                Written::Opened(deployment(1), terms(4900), at(21)),
                Written::Lifted(held.id, at(21)),
            ]
        );
        assert!(fixture.world.with(|world| {
            world
                .organisation
                .as_ref()
                .is_some_and(Organisation::is_suspended)
        }));
    }

    #[tokio::test]
    async fn an_organisation_someone_else_already_activated_is_not_saved_again() {
        let organisation = organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let held = suspension(organisation.id, uncollectible.id(), &[deployment(1)], true);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(held.clone())
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .restarting(&[deployment(1)]);

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written().last(),
            Some(&Written::Lifted(held.id, at(21)))
        );
    }

    #[tokio::test]
    async fn payment_failed_marks_the_statement_payment_failed_and_the_method_failing() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new()
            .with_statement(sent.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Valid));

        fixture
            .collection()
            .payment_failed(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(sent.id(), StatementStatus::PaymentFailed),
                Written::Account(PaymentMethodStatus::Failing),
            ]
        );
        assert_eq!(
            fixture.statement(sent.id()).status(),
            StatementStatus::PaymentFailed
        );
    }

    #[tokio::test]
    async fn payment_failed_on_a_statement_that_cannot_fail_leaves_it_alone() {
        let organisation = organisation();
        let paid = statement(organisation.id, "in_1", StatementStatus::Paid);
        let fixture = Fixture::new()
            .with_statement(paid.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Valid));

        fixture
            .collection()
            .payment_failed(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert!(fixture.written().is_empty());
        assert_eq!(fixture.statement(paid.id()).status(), StatementStatus::Paid);
    }

    #[tokio::test]
    async fn uncollectible_suspends_stops_closes_and_records_the_suspension() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let stopped = [deployment(1), deployment(2)];
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(sent.clone())
            .with_subscription(open_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(open_subscription(organisation.id, deployment(2), 9900))
            .with_subscription(open_subscription(organisation.id, deployment(3), 0))
            .stopping(&stopped);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(sent.id(), StatementStatus::Uncollectible),
                Written::Organisation(OrganisationStatus::Suspended),
                Written::Stopped(organisation.id),
                Written::Closed(deployment(1), Some(at(21))),
                Written::Closed(deployment(2), Some(at(21))),
                Written::Suspended {
                    statement: sent.id(),
                    stopped: stopped.to_vec(),
                    organisation: true,
                    at: at(21),
                    lifted: None,
                },
            ]
        );
        assert!(fixture.world.with(|world| {
            world
                .subscriptions
                .iter()
                .filter(|subscription| subscription.is_open())
                .map(Subscription::deployment_id)
                .collect::<Vec<_>>()
                == vec![deployment(3)]
        }));
    }

    #[tokio::test]
    async fn uncollectible_on_an_organisation_already_suspended_does_not_save_it_and_says_so() {
        let organisation = suspended_organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(sent.clone())
            .with_subscription(open_subscription(organisation.id, deployment(1), 4900))
            .stopping(&[deployment(1)]);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(sent.id(), StatementStatus::Uncollectible),
                Written::Stopped(organisation.id),
                Written::Closed(deployment(1), Some(at(21))),
                Written::Suspended {
                    statement: sent.id(),
                    stopped: vec![deployment(1)],
                    organisation: false,
                    at: at(21),
                    lifted: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn uncollectible_on_a_deleted_organisation_does_not_suspend_it() {
        let mut organisation = organisation();
        organisation.delete().expect("deleted");
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(sent.clone())
            .stopping(&[]);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert!(
            !fixture
                .written()
                .iter()
                .any(|written| matches!(written, Written::Organisation(..)))
        );
    }

    #[tokio::test]
    async fn uncollectible_skips_a_stopped_deployment_without_an_open_subscription() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let stopped = [deployment(1), deployment(2)];
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(sent.clone())
            .with_subscription(closed_subscription(organisation.id, deployment(1), 4900))
            .stopping(&stopped);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![
                Written::Statement(sent.id(), StatementStatus::Uncollectible),
                Written::Organisation(OrganisationStatus::Suspended),
                Written::Stopped(organisation.id),
                Written::Suspended {
                    statement: sent.id(),
                    stopped: stopped.to_vec(),
                    organisation: true,
                    at: at(21),
                    lifted: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn uncollectible_with_an_active_suspension_only_marks_the_statement() {
        let organisation = organisation();
        let earlier = statement(organisation.id, "in_0", StatementStatus::Uncollectible);
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(earlier.clone())
            .with_statement(sent.clone())
            .with_suspension(suspension(
                organisation.id,
                earlier.id(),
                &[deployment(1)],
                true,
            ))
            .stopping(&[deployment(1)]);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![Written::Statement(
                sent.id(),
                StatementStatus::Uncollectible
            )]
        );
    }

    #[tokio::test]
    async fn a_replayed_uncollectible_event_over_an_active_suspension_changes_nothing() {
        let organisation = organisation();
        let uncollectible = statement(organisation.id, "in_1", StatementStatus::Uncollectible);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(uncollectible.clone())
            .with_suspension(suspension(
                organisation.id,
                uncollectible.id(),
                &[deployment(1)],
                true,
            ))
            .stopping(&[deployment(1)]);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert!(fixture.written().is_empty());
    }

    #[tokio::test]
    async fn uncollectible_on_a_statement_already_paid_suspends_nothing() {
        let organisation = organisation();
        let paid = statement(organisation.id, "in_1", StatementStatus::Paid);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(paid)
            .with_subscription(open_subscription(organisation.id, deployment(1), 4900))
            .stopping(&[deployment(1)]);

        fixture
            .collection()
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("collected");

        assert!(fixture.written().is_empty());
    }

    #[tokio::test]
    async fn an_event_naming_an_unknown_invoice_changes_nothing_and_is_not_an_error() {
        let organisation = organisation();
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(statement(organisation.id, "in_1", StatementStatus::Sent))
            .with_account(account(organisation.id, PaymentMethodStatus::Valid))
            .stopping(&[deployment(1)]);
        let collection = fixture.collection();
        let unknown = invoice_id("in_unknown");

        collection
            .invoice_paid(&unknown, None, at(21))
            .await
            .expect("not an error");
        collection
            .payment_failed(&unknown, None, at(21))
            .await
            .expect("not an error");
        collection
            .uncollectible(&unknown, None, at(21))
            .await
            .expect("not an error");

        assert!(fixture.written().is_empty());
    }

    #[tokio::test]
    async fn an_event_naming_a_statement_the_platform_does_not_know_is_a_foreign_invoice() {
        let organisation = organisation();
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(statement(organisation.id, "in_1", StatementStatus::Sent))
            .with_account(account(organisation.id, PaymentMethodStatus::Valid))
            .stopping(&[deployment(1)]);
        let collection = fixture.collection();
        let unknown = invoice_id("in_unknown");
        let elsewhere = Some(StatementId::new());

        collection
            .invoice_paid(&unknown, elsewhere, at(21))
            .await
            .expect("not an error");
        collection
            .payment_failed(&unknown, elsewhere, at(21))
            .await
            .expect("not an error");
        collection
            .uncollectible(&unknown, elsewhere, at(21))
            .await
            .expect("not an error");

        assert!(fixture.written().is_empty());
    }

    #[tokio::test]
    async fn an_event_for_an_invoice_not_recorded_yet_asks_to_be_retried() {
        let organisation = organisation();
        let draft = unrecorded(organisation.id);
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(draft.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Failing))
            .stopping(&[deployment(1)]);
        let collection = fixture.collection();
        let early = invoice_id("in_early");
        let named = Some(draft.id());

        let outcomes = [
            collection.invoice_paid(&early, named, at(21)).await,
            collection.payment_failed(&early, named, at(21)).await,
            collection.uncollectible(&early, named, at(21)).await,
        ];

        for outcome in outcomes {
            assert!(
                matches!(
                    &outcome,
                    Err(CoreError::Billing(BillingError::InvoiceNotRecordedYet { invoice }))
                        if invoice == "in_early"
                ),
                "{outcome:?}"
            );
        }
        assert!(fixture.written().is_empty());
        assert_eq!(fixture.statement(draft.id()), draft);
    }

    #[tokio::test]
    async fn the_named_statement_is_not_asked_for_when_the_invoice_is_known() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let fixture = Fixture::new().with_statement(sent.clone());

        fixture
            .collection()
            .invoice_paid(&invoice_id("in_1"), Some(StatementId::new()), at(21))
            .await
            .expect("collected");

        assert_eq!(
            fixture.written(),
            vec![Written::Statement(sent.id(), StatementStatus::Paid)]
        );
    }

    #[tokio::test]
    async fn a_suspension_then_a_payment_restores_everything_and_deletes_nothing() {
        let organisation = organisation();
        let sent = statement(organisation.id, "in_1", StatementStatus::Sent);
        let stopped = [deployment(1), deployment(2)];
        let fixture = Fixture::new()
            .with_organisation(organisation.clone())
            .with_statement(sent.clone())
            .with_account(account(organisation.id, PaymentMethodStatus::Valid))
            .with_subscription(open_subscription(organisation.id, deployment(1), 4900))
            .with_subscription(open_subscription(organisation.id, deployment(2), 9900))
            .stopping(&stopped)
            .restarting(&stopped);
        let collection = fixture.collection();

        collection
            .uncollectible(&invoice_id("in_1"), None, at(21))
            .await
            .expect("suspended");
        collection
            .invoice_paid(&invoice_id("in_1"), None, at(23))
            .await
            .expect("restored");

        fixture.world.with(|world| {
            assert_eq!(world.statements.len(), 1);
            assert_eq!(world.statements[0].status(), StatementStatus::Paid);
            assert_eq!(world.suspensions.len(), 1);
            assert_eq!(world.suspensions[0].lifted_at, Some(at(23)));
            assert_eq!(
                world
                    .organisation
                    .as_ref()
                    .map(|organisation| organisation.status),
                Some(OrganisationStatus::Active)
            );
            assert_eq!(world.subscriptions.len(), 4);
            for id in stopped {
                let latest = world
                    .subscriptions
                    .iter()
                    .rev()
                    .find(|subscription| subscription.deployment_id() == id)
                    .expect("a subscription");
                assert!(latest.is_open());
                assert_eq!(latest.starts_at(), at(23));
            }
        });
    }

    fn adopting_accounts(
        held: Option<BillingAccount>,
        saved: &Arc<Mutex<Vec<&'static str>>>,
    ) -> MockBillingAccountRepository {
        let mut mock = MockBillingAccountRepository::new();
        mock.expect_find_by_customer().returning(move |_| {
            let held = held.clone();
            Box::pin(async move { Ok(held) })
        });
        let saved = Arc::clone(saved);
        mock.expect_save().returning(move |account| {
            saved.lock().expect("not poisoned").push(
                if account.payment_method == PaymentMethodStatus::Valid {
                    "save valid"
                } else {
                    "save other"
                },
            );
            Box::pin(async { Ok(()) })
        });
        mock
    }

    #[tokio::test]
    async fn adopting_a_payment_method_sets_the_default_then_saves_the_method_valid() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let accounts = adopting_accounts(
            Some(account(OrganisationId::new(), PaymentMethodStatus::Missing)),
            &calls,
        );
        let mut provider = MockPaymentProvider::new();
        let noted = Arc::clone(&calls);
        provider
            .expect_set_default_payment_method()
            .times(1)
            .withf(|customer, payment_method| {
                customer.as_str() == "cus_123" && payment_method.as_str() == "pm_1"
            })
            .returning(move |_, _| {
                noted.lock().expect("not poisoned").push("set default");
                Box::pin(async { Ok(()) })
            });

        adopt_payment_method(
            &accounts,
            &provider,
            &ProviderCustomerId::parse("cus_123").unwrap(),
            &ProviderPaymentMethodId::parse("pm_1").unwrap(),
        )
        .await
        .expect("adopted");

        assert_eq!(
            *calls.lock().expect("not poisoned"),
            vec!["set default", "save valid"]
        );
    }

    #[tokio::test]
    async fn adopting_for_an_unknown_customer_does_nothing_and_never_calls_the_provider() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let accounts = adopting_accounts(None, &calls);

        adopt_payment_method(
            &accounts,
            &MockPaymentProvider::new(),
            &ProviderCustomerId::parse("cus_unknown").unwrap(),
            &ProviderPaymentMethodId::parse("pm_1").unwrap(),
        )
        .await
        .expect("not an error");

        assert!(calls.lock().expect("not poisoned").is_empty());
    }

    #[tokio::test]
    async fn a_provider_refusing_the_default_method_saves_nothing() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let accounts = adopting_accounts(
            Some(account(OrganisationId::new(), PaymentMethodStatus::Missing)),
            &calls,
        );
        let mut provider = MockPaymentProvider::new();
        provider
            .expect_set_default_payment_method()
            .times(1)
            .returning(|_, _| {
                Box::pin(async {
                    Err(BillingError::Provider {
                        reason: "unreachable".to_string(),
                    }
                    .into())
                })
            });

        let failed = adopt_payment_method(
            &accounts,
            &provider,
            &ProviderCustomerId::parse("cus_123").unwrap(),
            &ProviderPaymentMethodId::parse("pm_1").unwrap(),
        )
        .await
        .expect_err("the provider failed");

        assert!(matches!(
            failed,
            CoreError::Billing(BillingError::Provider { .. })
        ));
        assert!(calls.lock().expect("not poisoned").is_empty());
    }
}
