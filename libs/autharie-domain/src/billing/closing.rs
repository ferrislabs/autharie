use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    billing::{
        BillingPeriod, Statement, StatementLine, estimate,
        ports::{BillingAccountRepository, StatementRepository, SubscriptionRepository},
        rate,
    },
    organisation::OrganisationId,
};

pub struct PeriodClose<S, A, ST> {
    subscriptions: S,
    accounts: A,
    statements: ST,
}

impl<S, A, ST> PeriodClose<S, A, ST>
where
    S: SubscriptionRepository,
    A: BillingAccountRepository,
    ST: StatementRepository,
{
    pub fn new(subscriptions: S, accounts: A, statements: ST) -> Self {
        Self {
            subscriptions,
            accounts,
            statements,
        }
    }

    pub async fn close(
        &self,
        organisation_id: OrganisationId,
        period: BillingPeriod,
        first_period: BillingPeriod,
        now: DateTime<Utc>,
    ) -> Result<Option<Statement>, CoreError> {
        if !period.has_ended(now) {
            return Ok(None);
        }

        if self
            .statements
            .find_by_period(&organisation_id, period)
            .await?
            .is_some()
        {
            return Ok(None);
        }

        let subscriptions = self
            .subscriptions
            .list_in_period(&organisation_id, period)
            .await?;
        let lines = rate(period, first_period, &subscriptions);
        let account = self.accounts.find(&organisation_id).await?;
        let Some(statement) =
            Statement::compose(organisation_id, period, lines, account.as_ref(), now)
        else {
            return Ok(None);
        };

        if !self.statements.insert(&statement).await? {
            return Ok(None);
        }

        Ok(Some(statement))
    }

    pub async fn estimate(
        &self,
        organisation_id: OrganisationId,
        first_period: BillingPeriod,
        now: DateTime<Utc>,
    ) -> Result<Vec<StatementLine>, CoreError> {
        let period = BillingPeriod::containing(now);
        let subscriptions = self
            .subscriptions
            .list_in_period(&organisation_id, period)
            .await?;

        Ok(estimate(period, first_period, &subscriptions, now))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{Arc, Mutex},
    };

    use chrono::TimeZone;
    use mockall::predicate::eq;
    use uuid::Uuid;

    use super::*;
    use crate::{
        billing::{
            BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
            BillingName, Money, PaymentMethodStatus, ProviderCustomerId, StatementStatus,
            Subscription, SubscriptionId, SubscriptionParts, SubscriptionTerms, TierCode,
            ports::{
                MockBillingAccountRepository, MockStatementRepository, MockSubscriptionRepository,
            },
        },
        deployments::DeploymentId,
        offers::Offer,
    };

    #[derive(Clone, Default)]
    struct Shelf(Arc<Mutex<Vec<Statement>>>);

    impl Shelf {
        fn put(&self, statement: &Statement) {
            self.0.lock().expect("not poisoned").push(statement.clone());
        }

        fn all(&self) -> Vec<Statement> {
            self.0.lock().expect("not poisoned").clone()
        }

        fn find(
            &self,
            organisation_id: &OrganisationId,
            period: BillingPeriod,
        ) -> Option<Statement> {
            self.all().into_iter().find(|statement| {
                statement.organisation_id() == *organisation_id && statement.period() == period
            })
        }
    }

    type Closing = PeriodClose<
        MockSubscriptionRepository,
        MockBillingAccountRepository,
        MockStatementRepository,
    >;

    struct Fixture {
        organisation_id: OrganisationId,
        held: Vec<Subscription>,
        account: Option<BillingAccount>,
        accepts: bool,
        shelf: Shelf,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                organisation_id: OrganisationId::new(),
                held: Vec::new(),
                account: None,
                accepts: true,
                shelf: Shelf::default(),
            }
        }

        fn holding(mut self, subscription: Subscription) -> Self {
            self.held.push(subscription);
            self
        }

        fn with_account(mut self, customer: bool, payment_method: PaymentMethodStatus) -> Self {
            self.account = Some(account(self.organisation_id, customer, payment_method));
            self
        }

        fn refusing_inserts(mut self) -> Self {
            self.accepts = false;
            self
        }

        fn closing(&self) -> Closing {
            PeriodClose::new(self.subscriptions(), self.accounts(), self.statements())
        }

        fn subscriptions(&self) -> MockSubscriptionRepository {
            let mut mock = MockSubscriptionRepository::new();
            let held = self.held.clone();
            mock.expect_list_in_period().returning(move |_, _| {
                let held = held.clone();
                Box::pin(async move { Ok(held) })
            });
            mock
        }

        fn accounts(&self) -> MockBillingAccountRepository {
            let mut mock = MockBillingAccountRepository::new();
            let account = self.account.clone();
            mock.expect_find().returning(move |_| {
                let account = account.clone();
                Box::pin(async move { Ok(account) })
            });
            mock
        }

        fn statements(&self) -> MockStatementRepository {
            let mut mock = MockStatementRepository::new();
            let shelf = self.shelf.clone();
            mock.expect_find_by_period()
                .returning(move |organisation_id, period| {
                    let found = shelf.find(organisation_id, period);
                    Box::pin(async move { Ok(found) })
                });
            let shelf = self.shelf.clone();
            let accepts = self.accepts;
            mock.expect_insert().returning(move |statement| {
                if accepts {
                    shelf.put(statement);
                }
                Box::pin(async move { Ok(accepts) })
            });
            mock
        }
    }

    fn at(month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, month, day, hour, 0, 0).unwrap()
    }

    fn november() -> BillingPeriod {
        BillingPeriod::new(2026, 11).unwrap()
    }

    fn january() -> BillingPeriod {
        BillingPeriod::new(2026, 1).unwrap()
    }

    fn month_end() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, 1, 0, 10, 0).unwrap()
    }

    fn terms(price: i64) -> SubscriptionTerms {
        SubscriptionTerms {
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(price),
            offer: Offer::Standard,
            features: BTreeSet::new(),
        }
    }

    fn deployment(n: u128) -> DeploymentId {
        DeploymentId(Uuid::from_u128(n))
    }

    fn subscription(
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        price: i64,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
    ) -> Subscription {
        Subscription::try_from(SubscriptionParts {
            id: SubscriptionId::new(),
            organisation_id,
            deployment_id,
            terms: terms(price),
            starts_at,
            ends_at,
        })
        .unwrap()
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
        customer: bool,
        payment_method: PaymentMethodStatus,
    ) -> BillingAccount {
        BillingAccount {
            customer: customer.then(|| ProviderCustomerId::parse("cus_123").unwrap()),
            payment_method,
            ..BillingAccount::new(organisation_id, details())
        }
    }

    fn paying(fixture: Fixture) -> Fixture {
        let organisation_id = fixture.organisation_id;
        fixture
            .holding(subscription(
                organisation_id,
                deployment(7),
                4900,
                at(10, 1, 0),
                None,
            ))
            .with_account(true, PaymentMethodStatus::Valid)
    }

    async fn close_november(fixture: &Fixture) -> Result<Option<Statement>, CoreError> {
        fixture
            .closing()
            .close(fixture.organisation_id, november(), january(), month_end())
            .await
    }

    #[tokio::test]
    async fn after_month_end_two_segments_and_another_deployment_make_one_statement_of_three_lines()
    {
        let fixture = Fixture::new();
        let organisation_id = fixture.organisation_id;
        let switch = at(11, 10, 8);
        let before = subscription(
            organisation_id,
            deployment(7),
            2900,
            at(10, 1, 0),
            Some(switch),
        );
        let after = before.successor(terms(9900), switch);
        let other = subscription(organisation_id, deployment(8), 4900, at(11, 1, 0), None);
        let fixture = fixture
            .holding(before)
            .holding(after)
            .holding(other)
            .with_account(true, PaymentMethodStatus::Valid);

        let statement = close_november(&fixture)
            .await
            .expect("closed")
            .expect("a statement");

        assert_eq!(statement.lines().len(), 3);
        assert_eq!(statement.total(), Money::from_minor(902 + 6820 + 4900));
        assert_eq!(statement.organisation_id(), organisation_id);
        assert_eq!(statement.period(), november());
        assert_eq!(statement.status(), StatementStatus::Draft);
        assert_eq!(statement.created_at(), month_end());
        assert_eq!(fixture.shelf.all(), vec![statement]);
    }

    #[tokio::test]
    async fn closing_the_same_period_again_gives_none_and_inserts_nothing() {
        let fixture = paying(Fixture::new());
        let closing = fixture.closing();

        let first = closing
            .close(fixture.organisation_id, november(), january(), month_end())
            .await
            .expect("closed");
        let again = closing
            .close(fixture.organisation_id, november(), january(), month_end())
            .await
            .expect("closed");

        assert!(first.is_some());
        assert_eq!(again, None);
        assert_eq!(fixture.shelf.all().len(), 1);
    }

    #[tokio::test]
    async fn only_free_subscriptions_give_no_statement() {
        let fixture = Fixture::new();
        let organisation_id = fixture.organisation_id;
        let fixture = fixture
            .holding(subscription(
                organisation_id,
                deployment(7),
                0,
                at(10, 1, 0),
                None,
            ))
            .with_account(true, PaymentMethodStatus::Valid);

        assert_eq!(close_november(&fixture).await.expect("closed"), None);
        assert!(fixture.shelf.all().is_empty());
    }

    #[tokio::test]
    async fn a_period_before_the_first_period_gives_no_statement() {
        let fixture = paying(Fixture::new());

        let closed = fixture
            .closing()
            .close(
                fixture.organisation_id,
                november(),
                BillingPeriod::new(2026, 12).unwrap(),
                month_end(),
            )
            .await
            .expect("closed");

        assert_eq!(closed, None);
        assert!(fixture.shelf.all().is_empty());
    }

    #[tokio::test]
    async fn a_period_that_has_not_ended_is_not_even_read() {
        let closing = PeriodClose::new(
            MockSubscriptionRepository::new(),
            MockBillingAccountRepository::new(),
            MockStatementRepository::new(),
        );

        let closed = closing
            .close(OrganisationId::new(), november(), january(), at(11, 30, 23))
            .await
            .expect("closed");

        assert_eq!(closed, None);
    }

    #[tokio::test]
    async fn a_period_is_closed_from_the_very_instant_it_ends() {
        let fixture = paying(Fixture::new());

        let closed = fixture
            .closing()
            .close(
                fixture.organisation_id,
                november(),
                january(),
                november().ends_at(),
            )
            .await
            .expect("closed");

        assert!(closed.is_some());
    }

    #[tokio::test]
    async fn an_insert_answering_false_gives_no_statement() {
        let fixture = paying(Fixture::new()).refusing_inserts();

        assert_eq!(close_november(&fixture).await.expect("closed"), None);
        assert!(fixture.shelf.all().is_empty());
    }

    #[tokio::test]
    async fn the_status_follows_what_the_account_can_do() {
        let cases = [
            (None, StatementStatus::Held),
            (
                Some((false, PaymentMethodStatus::Valid)),
                StatementStatus::Held,
            ),
            (
                Some((true, PaymentMethodStatus::Missing)),
                StatementStatus::Held,
            ),
            (
                Some((true, PaymentMethodStatus::Failing)),
                StatementStatus::Draft,
            ),
            (
                Some((true, PaymentMethodStatus::Valid)),
                StatementStatus::Draft,
            ),
        ];

        for (held, expected) in cases {
            let fixture = Fixture::new();
            let organisation_id = fixture.organisation_id;
            let mut fixture = fixture.holding(subscription(
                organisation_id,
                deployment(7),
                4900,
                at(10, 1, 0),
                None,
            ));
            if let Some((customer, payment_method)) = held {
                fixture = fixture.with_account(customer, payment_method);
            }

            let statement = close_november(&fixture)
                .await
                .expect("closed")
                .expect("a statement");

            assert_eq!(statement.status(), expected, "{held:?}");
        }
    }

    #[tokio::test]
    async fn a_statement_whose_lines_all_round_to_nothing_is_zero_whatever_the_account() {
        let fixture = Fixture::new();
        let organisation_id = fixture.organisation_id;
        let fixture = fixture
            .holding(subscription(
                organisation_id,
                deployment(7),
                100,
                at(11, 29, 0),
                Some(at(11, 29, 1)),
            ))
            .holding(subscription(
                organisation_id,
                deployment(8),
                100,
                at(11, 30, 0),
                Some(at(11, 30, 1)),
            ));

        let statement = close_november(&fixture)
            .await
            .expect("closed")
            .expect("a statement");

        assert_eq!(statement.lines().len(), 2);
        assert_eq!(statement.total(), Money::ZERO);
        assert_eq!(statement.status(), StatementStatus::Zero);
    }

    #[tokio::test]
    async fn the_estimate_cuts_the_current_month_at_now() {
        let fixture = paying(Fixture::new());
        let now = at(11, 16, 0);

        let lines = fixture
            .closing()
            .estimate(fixture.organisation_id, january(), now)
            .await
            .expect("estimated");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].starts_at, at(11, 1, 0));
        assert_eq!(lines[0].ends_at, now);
        assert_eq!(lines[0].covered_seconds, 15 * 86_400);
        assert_eq!(lines[0].amount, Money::from_minor(2450));
    }

    #[tokio::test]
    async fn the_estimate_reads_the_period_containing_now() {
        let organisation_id = OrganisationId::new();
        let mut subscriptions = MockSubscriptionRepository::new();
        subscriptions
            .expect_list_in_period()
            .with(eq(organisation_id), eq(november()))
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(Vec::new()) }));
        let closing = PeriodClose::new(
            subscriptions,
            MockBillingAccountRepository::new(),
            MockStatementRepository::new(),
        );

        let lines = closing
            .estimate(organisation_id, january(), at(11, 16, 0))
            .await
            .expect("estimated");

        assert!(lines.is_empty());
    }

    #[tokio::test]
    async fn the_estimate_of_a_period_before_the_first_period_is_empty() {
        let fixture = paying(Fixture::new());

        let lines = fixture
            .closing()
            .estimate(
                fixture.organisation_id,
                BillingPeriod::new(2026, 12).unwrap(),
                at(11, 16, 0),
            )
            .await
            .expect("estimated");

        assert!(lines.is_empty());
    }
}
