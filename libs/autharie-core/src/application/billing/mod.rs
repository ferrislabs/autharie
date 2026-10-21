use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    billing::{
        BillingError, BillingPeriod, ProviderEvent, ProviderEventKind, Statement, StatementStatus,
        Subscription, Tier, TierCode,
        closing::PeriodClose,
        collection::{Collection, adopt_payment_method},
        lifecycle::SubscriptionLifecycle,
        ports::{
            BillingPolicy, PaymentProvider, ProviderEventLog, StatementRepository,
            SubscriptionRepository, SubscriptionService, TierRepository,
        },
        sending::StatementSending,
    },
    deployments::{
        Deployment, DeploymentId,
        ports::{DeploymentPolicy, DeploymentRepository},
    },
    organisation::{OrganisationId, ports::OrganisationRepository},
};
use autharie_macros::transactional;
use chrono::{DateTime, Utc};
use tracing::error;

use crate::{AutharieService, infrastructure::role::permissions_in, policy::AuthariePolicy};

mod power;

use power::StoppingNothing;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BillingRun {
    pub closed: usize,
    pub sent: usize,
    pub failed: usize,
}

async fn live_deployment<D: DeploymentRepository>(
    deployments: &D,
    organisation_id: OrganisationId,
    deployment_id: DeploymentId,
) -> Result<Deployment, CoreError> {
    deployments
        .get_by_id(deployment_id)
        .await?
        .filter(|deployment| {
            deployment.organisation_id == organisation_id && deployment.deleted_at.is_none()
        })
        .ok_or(CoreError::DeploymentNotFound {
            id: deployment_id.0,
        })
}

impl SubscriptionService for AutharieService {
    #[transactional(tier)]
    async fn list_tiers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<Tier>, CoreError> {
        DeploymentPolicy::can_view_deployments(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity,
            organisation_id,
        )
        .await?;

        Ok(tier_repository
            .list()
            .await?
            .into_iter()
            .filter(|tier| tier.selectable)
            .collect())
    }

    #[transactional(deployment, subscription)]
    async fn subscription(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Subscription, CoreError> {
        DeploymentPolicy::can_view_deployments(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity,
            organisation_id,
        )
        .await?;

        let deployment =
            live_deployment(&deployment_repository, organisation_id, deployment_id).await?;

        Ok(subscription_repository
            .latest_for(&deployment.id)
            .await?
            .ok_or(BillingError::NoOpenSubscription {
                deployment: deployment.id.0,
            })?)
    }

    #[transactional(deployment, organisation, tier, subscription, billing_account)]
    async fn change_subscription(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        tier: TierCode,
    ) -> Result<Subscription, CoreError> {
        BillingPolicy::can_manage_billing(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity.clone(),
            organisation_id,
        )
        .await?;

        let deployment =
            live_deployment(&deployment_repository, organisation_id, deployment_id).await?;

        let organisation = organisation_repository
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })?;

        SubscriptionLifecycle::new(
            tier_repository,
            subscription_repository,
            billing_account_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .change(
            &identity,
            &organisation,
            &deployment,
            &tier,
            chrono::Utc::now(),
        )
        .await
    }
}

impl AutharieService {
    pub async fn run_billing<P: PaymentProvider>(
        &self,
        provider: &P,
        first_period: BillingPeriod,
        now: DateTime<Utc>,
    ) -> Result<BillingRun, CoreError> {
        let period = BillingPeriod::containing(now).previous();
        let mut run = BillingRun::default();

        if period >= first_period {
            for organisation_id in self.billable_organisations(period).await? {
                match self
                    .close_period(organisation_id, period, first_period, now)
                    .await
                {
                    Ok(Some(_)) => run.closed += 1,
                    Ok(None) => {}
                    Err(error) => {
                        run.failed += 1;
                        error!(
                            organisation = %organisation_id,
                            %period,
                            %error,
                            "could not close the billing period of an organisation"
                        );
                    }
                }
            }
        }

        for statement in self.unsent_statements().await? {
            let id = statement.id();

            match self.send_statement(provider, statement, now).await {
                Ok(sent) if sent.status() == StatementStatus::Sent => run.sent += 1,
                Ok(_) => {}
                Err(error) => {
                    run.failed += 1;
                    error!(statement = %id, %error, "could not send a statement");
                }
            }
        }

        Ok(run)
    }

    pub async fn apply_provider_event<P: PaymentProvider>(
        &self,
        provider: &P,
        payload: &[u8],
        signature: &str,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let event = provider.read_event(payload, signature, now)?;

        self.apply_verified_event(provider, event, now).await
    }

    #[transactional(
        provider_event,
        statement,
        billing_account,
        billing_suspension,
        subscription,
        organisation
    )]
    async fn apply_verified_event<P: PaymentProvider>(
        &self,
        provider: &P,
        event: ProviderEvent,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        if !provider_event_repository.record(&event.id, now).await? {
            return Ok(());
        }

        let collection = Collection {
            statements: statement_repository,
            accounts: billing_account_repository,
            suspensions: billing_suspension_repository,
            subscriptions: subscription_repository,
            organisations: organisation_repository,
            power: StoppingNothing,
        };

        match event.kind {
            ProviderEventKind::PaymentMethodReady {
                customer,
                payment_method,
            } => {
                adopt_payment_method(&collection.accounts, provider, &customer, &payment_method)
                    .await
            }
            ProviderEventKind::InvoicePaid { invoice, statement } => {
                collection.invoice_paid(&invoice, statement, now).await
            }
            ProviderEventKind::PaymentFailed { invoice, statement } => {
                collection.payment_failed(&invoice, statement, now).await
            }
            ProviderEventKind::InvoiceUncollectible { invoice, statement } => {
                collection.uncollectible(&invoice, statement, now).await
            }
            ProviderEventKind::Ignored => Ok(()),
        }
    }

    #[transactional(subscription)]
    async fn billable_organisations(
        &self,
        period: BillingPeriod,
    ) -> Result<Vec<OrganisationId>, CoreError> {
        subscription_repository
            .organisations_billable_in(period)
            .await
    }

    #[transactional(subscription, billing_account, statement)]
    async fn close_period(
        &self,
        organisation_id: OrganisationId,
        period: BillingPeriod,
        first_period: BillingPeriod,
        now: DateTime<Utc>,
    ) -> Result<Option<Statement>, CoreError> {
        PeriodClose::new(
            subscription_repository,
            billing_account_repository,
            statement_repository,
        )
        .close(organisation_id, period, first_period, now)
        .await
    }

    #[transactional(statement)]
    async fn unsent_statements(&self) -> Result<Vec<Statement>, CoreError> {
        statement_repository.list_unsent().await
    }

    #[transactional(billing_account, statement)]
    async fn send_statement<P: PaymentProvider>(
        &self,
        provider: &P,
        statement: Statement,
        now: DateTime<Utc>,
    ) -> Result<Statement, CoreError> {
        StatementSending::new(billing_account_repository, statement_repository, provider)
            .send_current(statement, now)
            .await
    }
}
