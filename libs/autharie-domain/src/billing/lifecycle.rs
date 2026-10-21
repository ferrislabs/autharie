use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    billing::{
        BillingError, Hosting, Subscription, SubscriptionTerms, Tier, TierCode,
        ports::{BillingAccountRepository, BillingPolicy, SubscriptionRepository, TierRepository},
        require_complete,
    },
    deployments::{Deployment, DeploymentId, DeploymentKind},
    offers::Offer,
    organisation::{Organisation, OrganisationId},
};

pub struct TierRequest<'a> {
    pub organisation: &'a Organisation,
    pub requested: Option<&'a TierCode>,
    pub offer: Offer,
    pub engine: &'a DeploymentKind,
    pub hosting: Hosting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierChoice {
    Catalogue(Tier),
    Legacy(Tier),
}

impl TierChoice {
    pub fn tier(&self) -> &Tier {
        match self {
            Self::Catalogue(tier) | Self::Legacy(tier) => tier,
        }
    }

    pub fn is_catalogue(&self) -> bool {
        matches!(self, Self::Catalogue(_))
    }
}

#[derive(Debug, Clone)]
pub enum Restorer {
    Staff,
    Member(Identity),
}

pub struct SubscriptionLifecycle<T, S, A, P> {
    tiers: T,
    subscriptions: S,
    accounts: A,
    policy: P,
}

impl<T, S, A, P> SubscriptionLifecycle<T, S, A, P>
where
    T: TierRepository,
    S: SubscriptionRepository,
    A: BillingAccountRepository,
    P: BillingPolicy,
{
    pub fn new(tiers: T, subscriptions: S, accounts: A, policy: P) -> Self {
        Self {
            tiers,
            subscriptions,
            accounts,
            policy,
        }
    }

    pub async fn choose(
        &self,
        identity: &Identity,
        request: TierRequest<'_>,
    ) -> Result<TierChoice, CoreError> {
        refuse_if_suspended(request.organisation)?;

        let any_selectable = self.tiers.list().await?.iter().any(|tier| tier.selectable);

        if !any_selectable {
            let tier = self.legacy_tier(request.offer).await?;

            if tier.is_billable() {
                self.require_paying(identity, request.organisation.id)
                    .await?;
            }

            return Ok(TierChoice::Legacy(tier));
        }

        let code = request.requested.ok_or(BillingError::TierRequired)?;
        let tier = self.tier(code).await?;
        require_selectable(&tier)?;
        tier.allows(request.engine, request.hosting)?;

        if tier.is_billable() {
            self.require_paying(identity, request.organisation.id)
                .await?;
        }

        Ok(TierChoice::Catalogue(tier))
    }

    pub async fn open(
        &self,
        deployment: &Deployment,
        terms: SubscriptionTerms,
        at: DateTime<Utc>,
    ) -> Result<Subscription, CoreError> {
        let subscription = Subscription::open(deployment.organisation_id, deployment.id, terms, at);
        self.subscriptions.open(&subscription).await?;

        Ok(subscription)
    }

    pub async fn change(
        &self,
        identity: &Identity,
        organisation: &Organisation,
        deployment: &Deployment,
        code: &TierCode,
        at: DateTime<Utc>,
    ) -> Result<Subscription, CoreError> {
        refuse_if_suspended(organisation)?;
        self.policy
            .can_manage_billing(identity.clone(), organisation.id)
            .await?;

        let tier = self.tier(code).await?;
        require_selectable(&tier)?;
        tier.allows(&deployment.kind, Hosting::of(&deployment.distribution))?;

        if tier.is_billable() {
            self.require_complete_account(&organisation.id).await?;
        }

        let current = self
            .subscriptions
            .latest_for(&deployment.id)
            .await?
            .filter(Subscription::is_open)
            .ok_or(BillingError::NoOpenSubscription {
                deployment: deployment.id.0,
            })?;

        if current.terms().tier == tier.code {
            return Ok(current);
        }

        let closed = current.close(at)?;
        self.subscriptions.close(&closed).await?;

        let next = closed.successor(tier.terms(), at);
        self.subscriptions.open(&next).await?;

        Ok(next)
    }

    pub async fn close_open(
        &self,
        deployment_id: DeploymentId,
        at: DateTime<Utc>,
    ) -> Result<Option<Subscription>, CoreError> {
        let Some(current) = self
            .subscriptions
            .latest_for(&deployment_id)
            .await?
            .filter(Subscription::is_open)
        else {
            return Ok(None);
        };

        let closed = current.close(at)?;
        self.subscriptions.close(&closed).await?;

        Ok(Some(closed))
    }

    pub async fn restore_terms(
        &self,
        restorer: &Restorer,
        source: DeploymentId,
        organisation_id: OrganisationId,
        offer: Offer,
    ) -> Result<SubscriptionTerms, CoreError> {
        let terms = match self.terms_of(source).await? {
            Some(terms) => terms,
            None => self.legacy_tier(offer).await?.terms(),
        };

        if let Restorer::Member(identity) = restorer
            && terms.is_billable()
        {
            self.require_paying(identity, organisation_id).await?;
        }

        Ok(terms)
    }

    pub async fn terms_of(
        &self,
        deployment_id: DeploymentId,
    ) -> Result<Option<SubscriptionTerms>, CoreError> {
        Ok(self
            .subscriptions
            .latest_for(&deployment_id)
            .await?
            .map(|subscription| subscription.terms().clone()))
    }

    async fn tier(&self, code: &TierCode) -> Result<Tier, CoreError> {
        self.tiers.find(code).await?.ok_or_else(|| {
            BillingError::TierNotFound {
                code: code.to_string(),
            }
            .into()
        })
    }

    async fn legacy_tier(&self, offer: Offer) -> Result<Tier, CoreError> {
        let code = TierCode::legacy(offer);

        self.tiers.find(&code).await?.ok_or_else(|| {
            CoreError::InternalError(format!("the {code} tier is missing from the catalogue"))
        })
    }

    async fn require_paying(
        &self,
        identity: &Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_billing(identity.clone(), organisation_id)
            .await?;

        self.require_complete_account(&organisation_id).await
    }

    async fn require_complete_account(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<(), CoreError> {
        let account = self.accounts.find(organisation_id).await?;
        require_complete(account.as_ref())?;

        Ok(())
    }
}

fn refuse_if_suspended(organisation: &Organisation) -> Result<(), CoreError> {
    if organisation.is_suspended() {
        return Err(CoreError::OrganisationSuspended {
            reason: "the organisation is suspended".to_string(),
        });
    }

    Ok(())
}

fn require_selectable(tier: &Tier) -> Result<(), CoreError> {
    if tier.selectable {
        return Ok(());
    }

    Err(BillingError::TierNotSelectable {
        code: tier.code.to_string(),
    }
    .into())
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
            BillingName, Money, PaymentMethodStatus,
            ports::{MockBillingAccountRepository, MockSubscriptionRepository, MockTierRepository},
        },
        dataplane::value_objects::{DataPlaneId, DeploymentResources},
        deployments::{DeploymentName, DeploymentStatus, distribution::Distribution},
        organisation::{
            features::IamFeature,
            value_objects::{OrganisationName, OrganisationSlug, Plan},
        },
        user::UserId,
        version::Version,
    };

    #[derive(Clone, Default)]
    struct Asked(Arc<Mutex<Vec<&'static str>>>);

    impl Asked {
        fn note(&self, what: &'static str) {
            self.0.lock().expect("not poisoned").push(what);
        }

        fn all(&self) -> Vec<&'static str> {
            self.0.lock().expect("not poisoned").clone()
        }
    }

    #[derive(Clone)]
    struct StubPolicy {
        allowed: bool,
        asked: Asked,
    }

    impl StubPolicy {
        fn answer(&self, right: &'static str) -> Result<(), CoreError> {
            self.asked.note(right);
            if self.allowed {
                Ok(())
            } else {
                Err(CoreError::PermissionDenied {
                    reason: "insufficient permissions".to_string(),
                })
            }
        }
    }

    impl BillingPolicy for StubPolicy {
        async fn can_view_billing(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            self.answer("view")
        }

        async fn can_manage_billing(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            self.answer("manage")
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Written {
        Opened(Subscription),
        Closed(Subscription),
    }

    #[derive(Clone, Default)]
    struct Ledger(Arc<Mutex<Vec<Written>>>);

    impl Ledger {
        fn record(&self, written: Written) {
            self.0.lock().expect("not poisoned").push(written);
        }

        fn all(&self) -> Vec<Written> {
            self.0.lock().expect("not poisoned").clone()
        }

        fn is_empty(&self) -> bool {
            self.all().is_empty()
        }

        fn opened(&self) -> Vec<Subscription> {
            self.all()
                .into_iter()
                .filter_map(|written| match written {
                    Written::Opened(subscription) => Some(subscription),
                    Written::Closed(_) => None,
                })
                .collect()
        }
    }

    type Lifecycle = SubscriptionLifecycle<
        MockTierRepository,
        MockSubscriptionRepository,
        MockBillingAccountRepository,
        StubPolicy,
    >;

    struct Fixture {
        catalogue: Vec<Tier>,
        held: Vec<Subscription>,
        account: Option<BillingAccount>,
        allowed: bool,
        asked: Asked,
        ledger: Ledger,
    }

    impl Fixture {
        fn new(catalogue: Vec<Tier>) -> Self {
            Self {
                catalogue,
                held: Vec::new(),
                account: None,
                allowed: true,
                asked: Asked::default(),
                ledger: Ledger::default(),
            }
        }

        fn holding(mut self, subscription: Subscription) -> Self {
            self.held.push(subscription);
            self
        }

        fn with_account(mut self, account: BillingAccount) -> Self {
            self.account = Some(account);
            self
        }

        fn refusing(mut self) -> Self {
            self.allowed = false;
            self
        }

        fn lifecycle(&self) -> Lifecycle {
            SubscriptionLifecycle::new(
                self.tiers(),
                self.subscriptions(),
                self.accounts(),
                StubPolicy {
                    allowed: self.allowed,
                    asked: self.asked.clone(),
                },
            )
        }

        fn tiers(&self) -> MockTierRepository {
            let mut mock = MockTierRepository::new();
            let listed = self.catalogue.clone();
            mock.expect_list().returning(move || {
                let listed = listed.clone();
                Box::pin(async move { Ok(listed) })
            });
            let known = self.catalogue.clone();
            mock.expect_find().returning(move |code| {
                let found = known.iter().find(|tier| &tier.code == code).cloned();
                Box::pin(async move { Ok(found) })
            });
            mock
        }

        fn subscriptions(&self) -> MockSubscriptionRepository {
            let mut mock = MockSubscriptionRepository::new();
            let held = self.held.clone();
            mock.expect_latest_for().returning(move |id| {
                let found = held
                    .iter()
                    .rev()
                    .find(|subscription| subscription.deployment_id() == *id)
                    .cloned();
                Box::pin(async move { Ok(found) })
            });
            let ledger = self.ledger.clone();
            mock.expect_open().returning(move |subscription| {
                ledger.record(Written::Opened(subscription.clone()));
                Box::pin(async { Ok(()) })
            });
            let ledger = self.ledger.clone();
            mock.expect_close().returning(move |subscription| {
                ledger.record(Written::Closed(subscription.clone()));
                Box::pin(async { Ok(()) })
            });
            mock
        }

        fn accounts(&self) -> MockBillingAccountRepository {
            let mut mock = MockBillingAccountRepository::new();
            let account = self.account.clone();
            let asked = self.asked.clone();
            mock.expect_find().returning(move |_| {
                asked.note("account");
                let account = account.clone();
                Box::pin(async move { Ok(account) })
            });
            mock
        }
    }

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
    }

    fn caller() -> Identity {
        Identity::User(autharie_auth::User {
            id: "user".to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })
    }

    fn tier(code: &str, price: i64) -> Tier {
        Tier {
            code: TierCode::parse(code).expect("a tier code"),
            name: code.to_string(),
            monthly_price: Money::from_minor(price),
            offer: Offer::Standard,
            features: BTreeSet::from([IamFeature::Mfa]),
            engines: vec![DeploymentKind::Ferriskey],
            hostings: vec![Hosting::Shared],
            selectable: true,
        }
    }

    fn free() -> Tier {
        tier("free-2026", 0)
    }

    fn pro() -> Tier {
        tier("pro-2026", 4900)
    }

    fn legacy(offer: Offer, price: i64) -> Tier {
        Tier {
            code: TierCode::legacy(offer),
            offer,
            selectable: false,
            engines: vec![DeploymentKind::Ferriskey, DeploymentKind::Keycloak],
            hostings: Hosting::ALL.to_vec(),
            ..tier("placeholder", price)
        }
    }

    fn organisation() -> Organisation {
        Organisation::new(
            OrganisationName::new("FerrisLabs").expect("a name"),
            OrganisationSlug::new("ferrislabs").expect("a slug"),
            UserId(Uuid::from_u128(7)),
            Plan::Free,
        )
    }

    fn suspended() -> Organisation {
        let mut organisation = organisation();
        organisation.suspend().expect("suspendable");
        organisation
    }

    fn deployment_of(organisation: &Organisation) -> Deployment {
        let created_at = at(3, 9);
        Deployment {
            topology: Default::default(),
            id: DeploymentId(Uuid::new_v4()),
            organisation_id: organisation.id,
            dataplane_id: DataPlaneId(Uuid::from_u128(3)),
            name: DeploymentName("auth".to_string()),
            kind: DeploymentKind::Ferriskey,
            version: Version::new(26, 0, 1),
            status: DeploymentStatus::Successful,
            namespace: "production-auth".to_string(),
            environment: crate::deployments::environment::Environment::Development,
            offer: None,
            restored_from: None,
            resources: DeploymentResources::DEFAULT,
            created_by: UserId(Uuid::from_u128(4)),
            created_at,
            updated_at: created_at,
            deployed_at: None,
            deleted_at: None,
            auto_upgrade: Default::default(),
            maintenance_window: None,
            network_access: crate::deployments::network::NetworkAccess::Open,
            last_verified_restore_at: None,
            last_restore_drill_seconds: None,
            log_shipping_enabled: false,
            iam_settings: Default::default(),
            distribution: Default::default(),
        }
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

    fn account(organisation: &Organisation, payment_method: PaymentMethodStatus) -> BillingAccount {
        BillingAccount {
            payment_method,
            ..BillingAccount::new(organisation.id, details())
        }
    }

    fn subscription_of(deployment: &Deployment, tier: &Tier, start: DateTime<Utc>) -> Subscription {
        Subscription::open(
            deployment.organisation_id,
            deployment.id,
            tier.terms(),
            start,
        )
    }

    fn request<'a>(
        organisation: &'a Organisation,
        requested: Option<&'a TierCode>,
        engine: &'a DeploymentKind,
        hosting: Hosting,
    ) -> TierRequest<'a> {
        TierRequest {
            organisation,
            requested,
            offer: Offer::Standard,
            engine,
            hosting,
        }
    }

    async fn subscribe(
        fixture: &Fixture,
        organisation: &Organisation,
        deployment: &Deployment,
        requested: Option<&TierCode>,
    ) -> Result<Subscription, CoreError> {
        let lifecycle = fixture.lifecycle();
        let choice = lifecycle
            .choose(
                &caller(),
                request(organisation, requested, &deployment.kind, Hosting::Shared),
            )
            .await?;

        lifecycle
            .open(deployment, choice.tier().terms(), deployment.created_at)
            .await
    }

    fn incomplete(error: &CoreError) -> bool {
        matches!(error, CoreError::Billing(BillingError::AccountIncomplete))
    }

    #[tokio::test]
    async fn a_free_tier_is_chosen_and_opened_without_an_account_or_the_right_to_manage_billing() {
        let fixture = Fixture::new(vec![free()]).refusing();
        let organisation = organisation();
        let deployment = deployment_of(&organisation);

        let opened = subscribe(&fixture, &organisation, &deployment, Some(&free().code))
            .await
            .expect("a free tier needs nothing");

        assert_eq!(opened.terms(), &free().terms());
        assert_eq!(fixture.ledger.opened(), vec![opened]);
        assert!(fixture.asked.all().is_empty());
    }

    #[tokio::test]
    async fn a_paid_tier_is_refused_while_the_account_is_missing_or_not_valid() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);

        for held in [
            None,
            Some(account(&organisation, PaymentMethodStatus::Missing)),
            Some(account(&organisation, PaymentMethodStatus::Failing)),
        ] {
            let mut fixture = Fixture::new(vec![pro()]);
            fixture.account = held.clone();

            let refused = subscribe(&fixture, &organisation, &deployment, Some(&pro().code))
                .await
                .expect_err("an incomplete account");

            assert!(incomplete(&refused), "{held:?}: {refused}");
            assert!(fixture.ledger.is_empty());
        }
    }

    #[tokio::test]
    async fn a_paid_tier_is_chosen_once_the_right_is_held_and_the_account_is_valid() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let fixture = Fixture::new(vec![pro()])
            .with_account(account(&organisation, PaymentMethodStatus::Valid));

        let opened = subscribe(&fixture, &organisation, &deployment, Some(&pro().code))
            .await
            .expect("a valid account");

        assert_eq!(opened.terms(), &pro().terms());
        assert_eq!(fixture.asked.all(), vec!["manage", "account"]);
    }

    #[tokio::test]
    async fn a_paid_tier_is_refused_to_a_caller_who_cannot_manage_billing() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let fixture = Fixture::new(vec![pro()])
            .with_account(account(&organisation, PaymentMethodStatus::Valid))
            .refusing();

        let refused = subscribe(&fixture, &organisation, &deployment, Some(&pro().code))
            .await
            .expect_err("no right to manage billing");

        assert!(matches!(refused, CoreError::PermissionDenied { .. }));
        assert_eq!(fixture.asked.all(), vec!["manage"]);
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn opening_after_creation_starts_at_the_creation_instant_and_copies_the_tier() {
        let fixture = Fixture::new(vec![pro()]);
        let organisation = organisation();
        let deployment = deployment_of(&organisation);

        let opened = fixture
            .lifecycle()
            .open(&deployment, pro().terms(), deployment.created_at)
            .await
            .expect("opened");

        assert_eq!(opened.starts_at(), at(3, 9));
        assert!(opened.is_open());
        assert_eq!(opened.organisation_id(), organisation.id);
        assert_eq!(opened.deployment_id(), deployment.id);
        assert_eq!(opened.terms().monthly_price, pro().monthly_price);
        assert_eq!(opened.terms().features, pro().features);
        assert_eq!(opened.terms().tier, pro().code);
        assert_eq!(fixture.ledger.all(), vec![Written::Opened(opened)]);
    }

    #[tokio::test]
    async fn a_tier_that_does_not_allow_the_engine_is_refused() {
        let fixture = Fixture::new(vec![free()]);
        let organisation = organisation();
        let code = free().code;

        let refused = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&code),
                    &DeploymentKind::Keycloak,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("keycloak is not listed");

        assert!(matches!(
            refused,
            CoreError::Billing(BillingError::TierDisallowsEngine { .. })
        ));
    }

    #[tokio::test]
    async fn a_tier_that_does_not_allow_the_hosting_is_refused() {
        let fixture = Fixture::new(vec![free()]);
        let organisation = organisation();
        let code = free().code;

        let refused = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&code),
                    &DeploymentKind::Ferriskey,
                    Hosting::CustomerCloud,
                ),
            )
            .await
            .expect_err("customer cloud is not listed");

        assert!(matches!(
            refused,
            CoreError::Billing(BillingError::TierDisallowsHosting { .. })
        ));
    }

    #[tokio::test]
    async fn an_unknown_or_unselectable_tier_is_refused() {
        let hidden = Tier {
            selectable: false,
            ..tier("hidden-2026", 0)
        };
        let fixture = Fixture::new(vec![free(), hidden.clone()]);
        let organisation = organisation();
        let unknown = TierCode::parse("ghost-2026").unwrap();

        let missing = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&unknown),
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("unknown");
        let closed = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&hidden.code),
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("unselectable");

        assert!(matches!(
            missing,
            CoreError::Billing(BillingError::TierNotFound { .. })
        ));
        assert!(matches!(
            closed,
            CoreError::Billing(BillingError::TierNotSelectable { .. })
        ));
    }

    #[tokio::test]
    async fn a_change_closes_the_current_subscription_and_opens_the_next_at_the_same_instant() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let current = subscription_of(&deployment, &free(), at(1, 0));
        let fixture = Fixture::new(vec![free(), pro()])
            .with_account(account(&organisation, PaymentMethodStatus::Valid))
            .holding(current.clone());

        let next = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &pro().code,
                at(10, 12),
            )
            .await
            .expect("changed");

        let closed = current.close(at(10, 12)).unwrap();
        assert_eq!(closed.ends_at(), Some(at(10, 12)));
        assert_eq!(next.starts_at(), at(10, 12));
        assert_eq!(next.terms(), &pro().terms());
        assert_eq!(next.deployment_id(), deployment.id);
        assert!(next.is_open());
        assert_eq!(
            fixture.ledger.all(),
            vec![Written::Closed(closed), Written::Opened(next)]
        );
    }

    #[tokio::test]
    async fn a_change_to_a_paid_tier_is_refused_while_the_account_is_incomplete() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let fixture = Fixture::new(vec![free(), pro()])
            .with_account(account(&organisation, PaymentMethodStatus::Missing))
            .holding(subscription_of(&deployment, &free(), at(1, 0)));

        let refused = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &pro().code,
                at(10, 12),
            )
            .await
            .expect_err("incomplete account");

        assert!(incomplete(&refused), "{refused}");
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_change_needs_the_right_to_manage_billing_whatever_the_price() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let fixture = Fixture::new(vec![free(), pro()])
            .holding(subscription_of(&deployment, &pro(), at(1, 0)))
            .refusing();

        let refused = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &free().code,
                at(10, 12),
            )
            .await
            .expect_err("no right to manage billing");

        assert!(matches!(refused, CoreError::PermissionDenied { .. }));
        assert_eq!(fixture.asked.all(), vec!["manage"]);
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_change_of_a_suspended_organisation_is_refused() {
        let organisation = suspended();
        let deployment = deployment_of(&organisation);
        let fixture = Fixture::new(vec![free(), pro()]).holding(subscription_of(
            &deployment,
            &pro(),
            at(1, 0),
        ));

        let refused = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &free().code,
                at(10, 12),
            )
            .await
            .expect_err("suspended");

        assert!(
            matches!(
                &refused,
                CoreError::OrganisationSuspended { reason } if reason == "the organisation is suspended"
            ),
            "{refused}"
        );
        assert!(fixture.asked.all().is_empty());
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_change_without_an_open_subscription_is_refused() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let closed = subscription_of(&deployment, &free(), at(1, 0))
            .close(at(2, 0))
            .unwrap();

        for fixture in [
            Fixture::new(vec![free(), pro()]),
            Fixture::new(vec![free(), pro()]).holding(closed.clone()),
        ] {
            let refused = fixture
                .lifecycle()
                .change(
                    &caller(),
                    &organisation,
                    &deployment,
                    &free().code,
                    at(10, 12),
                )
                .await
                .expect_err("nothing open");

            assert!(
                matches!(
                    &refused,
                    CoreError::Billing(BillingError::NoOpenSubscription { deployment: id })
                        if *id == deployment.id.0
                ),
                "{refused}"
            );
            assert!(fixture.ledger.is_empty());
        }
    }

    #[tokio::test]
    async fn a_change_to_the_current_tier_writes_nothing() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let current = subscription_of(&deployment, &pro(), at(1, 0));
        let fixture = Fixture::new(vec![free(), pro()])
            .with_account(account(&organisation, PaymentMethodStatus::Valid))
            .holding(current.clone());

        let same = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &pro().code,
                at(10, 12),
            )
            .await
            .expect("nothing to do");

        assert_eq!(same, current);
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_change_to_a_tier_that_cannot_be_selected_or_hosted_is_refused() {
        let organisation = organisation();
        let mut deployment = deployment_of(&organisation);
        let hidden = Tier {
            selectable: false,
            ..tier("hidden-2026", 0)
        };
        let fixture = Fixture::new(vec![free(), hidden.clone()]).holding(subscription_of(
            &deployment,
            &free(),
            at(1, 0),
        ));
        let unknown = TierCode::parse("ghost-2026").unwrap();

        let missing = fixture
            .lifecycle()
            .change(&caller(), &organisation, &deployment, &unknown, at(10, 12))
            .await
            .expect_err("unknown");
        let closed = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &hidden.code,
                at(10, 12),
            )
            .await
            .expect_err("unselectable");
        deployment.distribution = Distribution::SelfHosted;
        let elsewhere = fixture
            .lifecycle()
            .change(
                &caller(),
                &organisation,
                &deployment,
                &free().code,
                at(10, 12),
            )
            .await
            .expect_err("self hosting is not listed");

        assert!(matches!(
            missing,
            CoreError::Billing(BillingError::TierNotFound { .. })
        ));
        assert!(matches!(
            closed,
            CoreError::Billing(BillingError::TierNotSelectable { .. })
        ));
        assert!(matches!(
            elsewhere,
            CoreError::Billing(BillingError::TierDisallowsHosting { .. })
        ));
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn while_no_tier_is_selectable_a_free_legacy_tier_is_chosen_without_a_gate() {
        let hidden = Tier {
            selectable: false,
            ..pro()
        };
        let fixture = Fixture::new(vec![legacy(Offer::Standard, 0), hidden.clone()]).refusing();
        let organisation = organisation();

        let choice = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&hidden.code),
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect("legacy");

        assert!(!choice.is_catalogue());
        assert_eq!(choice.tier().code.as_str(), "legacy-standard");
        assert!(fixture.asked.all().is_empty());
    }

    #[tokio::test]
    async fn while_no_tier_is_selectable_a_priced_legacy_tier_keeps_the_paid_gate() {
        let hidden = Tier {
            selectable: false,
            ..pro()
        };
        let organisation = organisation();
        let fixture = Fixture::new(vec![legacy(Offer::Standard, 4900), hidden.clone()])
            .with_account(account(&organisation, PaymentMethodStatus::Valid));

        let choice = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&hidden.code),
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect("legacy");

        assert!(!choice.is_catalogue());
        assert_eq!(choice.tier().code.as_str(), "legacy-standard");
        assert_eq!(fixture.asked.all(), vec!["manage", "account"]);
    }

    #[tokio::test]
    async fn while_no_tier_is_selectable_a_priced_legacy_tier_is_refused_to_a_caller_who_cannot_manage_billing()
     {
        let organisation = organisation();
        let fixture = Fixture::new(vec![legacy(Offer::Standard, 4900)])
            .with_account(account(&organisation, PaymentMethodStatus::Valid))
            .refusing();

        let refused = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    None,
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("no right to manage billing");

        assert!(matches!(refused, CoreError::PermissionDenied { .. }));
        assert_eq!(fixture.asked.all(), vec!["manage"]);
    }

    #[tokio::test]
    async fn while_no_tier_is_selectable_a_priced_legacy_tier_is_refused_while_the_account_is_incomplete()
     {
        let organisation = organisation();

        for held in [
            None,
            Some(account(&organisation, PaymentMethodStatus::Missing)),
            Some(account(&organisation, PaymentMethodStatus::Failing)),
        ] {
            let mut fixture = Fixture::new(vec![legacy(Offer::Standard, 4900)]);
            fixture.account = held.clone();

            let refused = fixture
                .lifecycle()
                .choose(
                    &caller(),
                    request(
                        &organisation,
                        None,
                        &DeploymentKind::Ferriskey,
                        Hosting::Shared,
                    ),
                )
                .await
                .expect_err("an incomplete account");

            assert!(incomplete(&refused), "{held:?}: {refused}");
            assert_eq!(fixture.asked.all(), vec!["manage", "account"]);
        }
    }

    #[tokio::test]
    async fn while_no_tier_is_selectable_a_missing_legacy_tier_is_an_internal_error() {
        let fixture = Fixture::new(Vec::new());
        let organisation = organisation();

        let refused = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    None,
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("no legacy tier");

        assert!(
            matches!(
                &refused,
                CoreError::InternalError(message)
                    if message == "the legacy-standard tier is missing from the catalogue"
            ),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn once_a_tier_is_selectable_a_request_without_a_tier_is_refused() {
        let fixture = Fixture::new(vec![legacy(Offer::Standard, 4900), free()]);
        let organisation = organisation();

        let refused = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    None,
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("a tier is required");

        assert!(matches!(
            refused,
            CoreError::Billing(BillingError::TierRequired)
        ));
    }

    #[tokio::test]
    async fn a_chosen_catalogue_tier_says_so() {
        let fixture = Fixture::new(vec![free()]);
        let organisation = organisation();
        let code = free().code;

        let choice = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&code),
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect("chosen");

        assert!(choice.is_catalogue());
        assert_eq!(choice, TierChoice::Catalogue(free()));
    }

    #[tokio::test]
    async fn editing_the_catalogue_changes_no_existing_subscription() {
        let mut edited = pro();
        let fixture = Fixture::new(vec![edited.clone()]);
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let opened = fixture
            .lifecycle()
            .open(&deployment, edited.terms(), deployment.created_at)
            .await
            .expect("opened");
        let before = opened.terms().clone();

        edited.monthly_price = Money::from_minor(9900);
        edited.features.insert(IamFeature::Branding);
        edited.offer = Offer::Scale;

        assert_eq!(opened.terms(), &before);
        assert_eq!(opened.terms().monthly_price, Money::from_minor(4900));
        assert!(!opened.terms().opens(IamFeature::Branding));
        assert_ne!(edited.terms(), before);
    }

    #[tokio::test]
    async fn a_suspended_organisation_cannot_create_but_a_restore_still_opens() {
        let organisation = suspended();
        let source = deployment_of(&organisation);
        let target = deployment_of(&organisation);
        let fixture =
            Fixture::new(vec![free()]).holding(subscription_of(&source, &free(), at(1, 0)));
        let code = free().code;

        let refused = fixture
            .lifecycle()
            .choose(
                &caller(),
                request(
                    &organisation,
                    Some(&code),
                    &DeploymentKind::Ferriskey,
                    Hosting::Shared,
                ),
            )
            .await
            .expect_err("suspended");
        let lifecycle = fixture.lifecycle();
        let terms = lifecycle
            .restore_terms(
                &Restorer::Staff,
                source.id,
                organisation.id,
                Offer::Standard,
            )
            .await
            .expect("a restore is not refused for that");
        let restored = lifecycle
            .open(&target, terms, at(8, 0))
            .await
            .expect("opened");

        assert!(
            matches!(
                &refused,
                CoreError::OrganisationSuspended { reason } if reason == "the organisation is suspended"
            ),
            "{refused}"
        );
        assert_eq!(restored.deployment_id(), target.id);
        assert_eq!(fixture.ledger.opened(), vec![restored]);
    }

    #[tokio::test]
    async fn closing_ends_the_open_subscription_at_the_given_instant() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let current = subscription_of(&deployment, &pro(), at(1, 0));
        let fixture = Fixture::new(vec![pro()]).holding(current.clone());

        let closed = fixture
            .lifecycle()
            .close_open(deployment.id, at(5, 6))
            .await
            .expect("closed")
            .expect("something was open");

        assert_eq!(closed.id(), current.id());
        assert_eq!(closed.ends_at(), Some(at(5, 6)));
        assert_eq!(fixture.ledger.all(), vec![Written::Closed(closed)]);
    }

    #[tokio::test]
    async fn closing_when_nothing_is_open_answers_none_and_writes_nothing() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let closed = subscription_of(&deployment, &pro(), at(1, 0))
            .close(at(2, 0))
            .unwrap();

        for fixture in [
            Fixture::new(vec![pro()]),
            Fixture::new(vec![pro()]).holding(closed.clone()),
        ] {
            let answer = fixture
                .lifecycle()
                .close_open(deployment.id, at(5, 6))
                .await
                .expect("nothing to fail");

            assert_eq!(answer, None);
            assert!(fixture.ledger.is_empty());
        }
    }

    fn member() -> Restorer {
        Restorer::Member(caller())
    }

    #[tokio::test]
    async fn a_restore_copies_the_terms_of_the_latest_subscription_of_the_source_and_writes_nothing()
     {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let earlier = subscription_of(&source, &free(), at(1, 0))
            .close(at(2, 0))
            .unwrap();
        let latest = subscription_of(&source, &pro(), at(2, 0));
        let fixture = Fixture::new(vec![free(), pro(), legacy(Offer::Standard, 4900)])
            .holding(earlier)
            .holding(latest.clone());

        let terms = fixture
            .lifecycle()
            .restore_terms(&Restorer::Staff, source.id, organisation.id, Offer::Scale)
            .await
            .expect("restored");

        assert_eq!(&terms, latest.terms());
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_member_restoring_paid_terms_is_refused_without_the_right_to_manage_billing() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(vec![pro()])
            .holding(subscription_of(&source, &pro(), at(1, 0)))
            .with_account(account(&organisation, PaymentMethodStatus::Valid))
            .refusing();

        let refused = fixture
            .lifecycle()
            .restore_terms(&member(), source.id, organisation.id, Offer::Standard)
            .await
            .expect_err("no right to manage billing");

        assert!(matches!(refused, CoreError::PermissionDenied { .. }));
        assert_eq!(fixture.asked.all(), vec!["manage"]);
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_member_restoring_paid_terms_is_refused_while_the_account_is_incomplete() {
        let organisation = organisation();
        let source = deployment_of(&organisation);

        for held in [
            None,
            Some(account(&organisation, PaymentMethodStatus::Missing)),
            Some(account(&organisation, PaymentMethodStatus::Failing)),
        ] {
            let mut fixture =
                Fixture::new(vec![pro()]).holding(subscription_of(&source, &pro(), at(1, 0)));
            fixture.account = held.clone();

            let refused = fixture
                .lifecycle()
                .restore_terms(&member(), source.id, organisation.id, Offer::Standard)
                .await
                .expect_err("the account cannot pay");

            assert!(incomplete(&refused), "{held:?}: {refused}");
            assert_eq!(fixture.asked.all(), vec!["manage", "account"]);
            assert!(fixture.ledger.is_empty());
        }
    }

    #[tokio::test]
    async fn a_member_restoring_paid_terms_succeeds_with_the_right_and_a_complete_account() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(vec![pro()])
            .holding(subscription_of(&source, &pro(), at(1, 0)))
            .with_account(account(&organisation, PaymentMethodStatus::Valid));

        let terms = fixture
            .lifecycle()
            .restore_terms(&member(), source.id, organisation.id, Offer::Standard)
            .await
            .expect("a complete account");

        assert_eq!(terms, pro().terms());
        assert_eq!(fixture.asked.all(), vec!["manage", "account"]);
    }

    #[tokio::test]
    async fn staff_restoring_paid_terms_is_asked_for_nothing() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(vec![pro()])
            .holding(subscription_of(&source, &pro(), at(1, 0)))
            .with_account(account(&organisation, PaymentMethodStatus::Failing))
            .refusing();

        let terms = fixture
            .lifecycle()
            .restore_terms(
                &Restorer::Staff,
                source.id,
                organisation.id,
                Offer::Standard,
            )
            .await
            .expect("staff restores whatever the account");

        assert_eq!(terms, pro().terms());
        assert!(fixture.asked.all().is_empty());
    }

    #[tokio::test]
    async fn a_member_restoring_free_terms_needs_no_right_and_no_account() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(vec![free()])
            .holding(subscription_of(&source, &free(), at(1, 0)))
            .refusing();

        let terms = fixture
            .lifecycle()
            .restore_terms(&member(), source.id, organisation.id, Offer::Standard)
            .await
            .expect("free terms");

        assert_eq!(terms, free().terms());
        assert!(fixture.asked.all().is_empty());
    }

    #[tokio::test]
    async fn a_restore_without_a_source_subscription_falls_back_to_the_legacy_tier_of_the_offer() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(vec![
            legacy(Offer::Standard, 4900),
            legacy(Offer::Scale, 9900),
        ]);

        let scale = fixture
            .lifecycle()
            .restore_terms(&Restorer::Staff, source.id, organisation.id, Offer::Scale)
            .await
            .expect("legacy scale");
        let standard = fixture
            .lifecycle()
            .restore_terms(
                &Restorer::Staff,
                source.id,
                organisation.id,
                Offer::Standard,
            )
            .await
            .expect("legacy standard");

        assert_eq!(scale, legacy(Offer::Scale, 9900).terms());
        assert_eq!(standard, legacy(Offer::Standard, 4900).terms());
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_member_restoring_legacy_paid_terms_goes_through_the_paid_gate() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(vec![legacy(Offer::Standard, 4900)]);

        let refused = fixture
            .lifecycle()
            .restore_terms(&member(), source.id, organisation.id, Offer::Standard)
            .await
            .expect_err("no account");

        assert!(incomplete(&refused), "{refused}");
        assert_eq!(fixture.asked.all(), vec!["manage", "account"]);
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn a_restore_without_a_source_subscription_or_a_legacy_tier_is_an_internal_error() {
        let organisation = organisation();
        let source = deployment_of(&organisation);
        let fixture = Fixture::new(Vec::new());

        let refused = fixture
            .lifecycle()
            .restore_terms(&Restorer::Staff, source.id, organisation.id, Offer::Scale)
            .await
            .expect_err("nothing to copy");

        assert!(
            matches!(
                &refused,
                CoreError::InternalError(message)
                    if message == "the legacy-scale tier is missing from the catalogue"
            ),
            "{refused}"
        );
        assert!(fixture.ledger.is_empty());
    }

    #[tokio::test]
    async fn the_terms_of_a_deployment_are_those_of_its_latest_subscription() {
        let organisation = organisation();
        let deployment = deployment_of(&organisation);
        let other = deployment_of(&organisation);
        let fixture = Fixture::new(vec![free(), pro()])
            .holding(
                subscription_of(&deployment, &free(), at(1, 0))
                    .close(at(2, 0))
                    .unwrap(),
            )
            .holding(subscription_of(&deployment, &pro(), at(2, 0)));

        let latest = fixture.lifecycle().terms_of(deployment.id).await.unwrap();
        let none = fixture.lifecycle().terms_of(other.id).await.unwrap();

        assert_eq!(latest, Some(pro().terms()));
        assert_eq!(none, None);
        assert!(fixture.ledger.is_empty());
    }
}
