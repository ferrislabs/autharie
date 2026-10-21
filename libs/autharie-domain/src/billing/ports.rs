use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use crate::{
    CoreError,
    billing::{
        BillingAccount, BillingDetails, BillingError, BillingPeriod, BillingSuspension,
        BillingSuspensionId, CardSummary, ProviderCustomerId, ProviderEvent, ProviderEventId,
        ProviderInvoice, ProviderInvoiceId, ProviderPaymentMethodId, Statement, StatementId,
        Subscription, Tier, TierCode,
    },
    deployments::DeploymentId,
    organisation::OrganisationId,
};

#[cfg_attr(test, mockall::automock)]
pub trait TierRepository: Send + Sync {
    fn list(&self) -> impl Future<Output = Result<Vec<Tier>, CoreError>> + Send;

    fn find(&self, code: &TierCode)
    -> impl Future<Output = Result<Option<Tier>, CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait SubscriptionRepository: Send + Sync {
    fn open(
        &self,
        subscription: &Subscription,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn close(
        &self,
        subscription: &Subscription,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn latest_for(
        &self,
        deployment_id: &DeploymentId,
    ) -> impl Future<Output = Result<Option<Subscription>, CoreError>> + Send;

    fn list_in_period(
        &self,
        organisation_id: &OrganisationId,
        period: BillingPeriod,
    ) -> impl Future<Output = Result<Vec<Subscription>, CoreError>> + Send;

    fn organisations_billable_in(
        &self,
        period: BillingPeriod,
    ) -> impl Future<Output = Result<Vec<OrganisationId>, CoreError>> + Send;

    fn list_open_billable(
        &self,
        organisation_id: &OrganisationId,
    ) -> impl Future<Output = Result<Vec<Subscription>, CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait BillingAccountRepository: Send + Sync {
    fn find(
        &self,
        organisation_id: &OrganisationId,
    ) -> impl Future<Output = Result<Option<BillingAccount>, CoreError>> + Send;

    fn find_by_customer(
        &self,
        customer: &ProviderCustomerId,
    ) -> impl Future<Output = Result<Option<BillingAccount>, CoreError>> + Send;

    fn save(&self, account: &BillingAccount) -> impl Future<Output = Result<(), CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait StatementRepository: Send + Sync {
    fn insert(&self, statement: &Statement)
    -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn update(&self, statement: &Statement) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn find(
        &self,
        id: &StatementId,
    ) -> impl Future<Output = Result<Option<Statement>, CoreError>> + Send;

    fn find_by_period(
        &self,
        organisation_id: &OrganisationId,
        period: BillingPeriod,
    ) -> impl Future<Output = Result<Option<Statement>, CoreError>> + Send;

    fn find_by_invoice(
        &self,
        invoice: &ProviderInvoiceId,
    ) -> impl Future<Output = Result<Option<Statement>, CoreError>> + Send;

    fn list_for(
        &self,
        organisation_id: &OrganisationId,
    ) -> impl Future<Output = Result<Vec<Statement>, CoreError>> + Send;

    fn list_unsent(&self) -> impl Future<Output = Result<Vec<Statement>, CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait ProviderEventLog: Send + Sync {
    fn record(
        &self,
        id: &ProviderEventId,
        received_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait BillingSuspensionRepository: Send + Sync {
    fn insert(
        &self,
        suspension: &BillingSuspension,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn find_active(
        &self,
        organisation_id: &OrganisationId,
    ) -> impl Future<Output = Result<Option<BillingSuspension>, CoreError>> + Send;

    fn list_active(&self)
    -> impl Future<Output = Result<Vec<BillingSuspension>, CoreError>> + Send;

    fn add_stopped(
        &self,
        id: &BillingSuspensionId,
        deployments: &[DeploymentId],
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn lift(
        &self,
        id: &BillingSuspensionId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait PaymentProvider: Send + Sync {
    fn create_customer(
        &self,
        organisation_id: &OrganisationId,
        details: &BillingDetails,
    ) -> impl Future<Output = Result<ProviderCustomerId, CoreError>> + Send;

    fn update_customer(
        &self,
        customer: &ProviderCustomerId,
        details: &BillingDetails,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn payment_method_session(
        &self,
        customer: &ProviderCustomerId,
        return_url: &str,
    ) -> impl Future<Output = Result<String, CoreError>> + Send;

    fn set_default_payment_method(
        &self,
        customer: &ProviderCustomerId,
        payment_method: &ProviderPaymentMethodId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn payment_method_card(
        &self,
        payment_method: &ProviderPaymentMethodId,
    ) -> impl Future<Output = Result<Option<CardSummary>, CoreError>> + Send;

    fn issue_invoice(
        &self,
        customer: &ProviderCustomerId,
        statement: &Statement,
    ) -> impl Future<Output = Result<ProviderInvoice, CoreError>> + Send;

    fn read_event(
        &self,
        payload: &[u8],
        signature: &str,
        now: DateTime<Utc>,
    ) -> Result<ProviderEvent, BillingError>;
}

#[cfg_attr(test, mockall::automock)]
pub trait DeploymentPower: Send + Sync {
    fn stop_billable(
        &self,
        organisation_id: &OrganisationId,
    ) -> impl Future<Output = Result<Vec<DeploymentId>, CoreError>> + Send;

    fn start(
        &self,
        deployment_ids: &[DeploymentId],
    ) -> impl Future<Output = Result<Vec<DeploymentId>, CoreError>> + Send;
}

impl<T: PaymentProvider> PaymentProvider for &T {
    fn create_customer(
        &self,
        organisation_id: &OrganisationId,
        details: &BillingDetails,
    ) -> impl Future<Output = Result<ProviderCustomerId, CoreError>> + Send {
        (**self).create_customer(organisation_id, details)
    }

    fn update_customer(
        &self,
        customer: &ProviderCustomerId,
        details: &BillingDetails,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        (**self).update_customer(customer, details)
    }

    fn payment_method_session(
        &self,
        customer: &ProviderCustomerId,
        return_url: &str,
    ) -> impl Future<Output = Result<String, CoreError>> + Send {
        (**self).payment_method_session(customer, return_url)
    }

    fn set_default_payment_method(
        &self,
        customer: &ProviderCustomerId,
        payment_method: &ProviderPaymentMethodId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        (**self).set_default_payment_method(customer, payment_method)
    }

    fn payment_method_card(
        &self,
        payment_method: &ProviderPaymentMethodId,
    ) -> impl Future<Output = Result<Option<CardSummary>, CoreError>> + Send {
        (**self).payment_method_card(payment_method)
    }

    fn issue_invoice(
        &self,
        customer: &ProviderCustomerId,
        statement: &Statement,
    ) -> impl Future<Output = Result<ProviderInvoice, CoreError>> + Send {
        (**self).issue_invoice(customer, statement)
    }

    fn read_event(
        &self,
        payload: &[u8],
        signature: &str,
        now: DateTime<Utc>,
    ) -> Result<ProviderEvent, BillingError> {
        (**self).read_event(payload, signature, now)
    }
}

pub trait BillingPolicy: Send + Sync {
    fn can_view_billing(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn can_manage_billing(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait SubscriptionService: Send + Sync {
    fn list_tiers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<Tier>, CoreError>> + Send;

    fn subscription(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Subscription, CoreError>> + Send;

    fn change_subscription(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        tier: TierCode,
    ) -> impl Future<Output = Result<Subscription, CoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use mockall::predicate::eq;

    use super::*;
    use crate::billing::ProviderEventKind;

    async fn count_unsent(statements: &impl StatementRepository) -> Result<usize, CoreError> {
        Ok(statements.list_unsent().await?.len())
    }

    async fn is_new_event(
        log: &impl ProviderEventLog,
        id: &ProviderEventId,
        now: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        log.record(id, now).await
    }

    fn read(
        provider: &impl PaymentProvider,
        payload: &[u8],
        now: DateTime<Utc>,
    ) -> Result<ProviderEvent, CoreError> {
        Ok(provider.read_event(payload, "t=1,v1=abc", now)?)
    }

    #[tokio::test]
    async fn a_statement_repository_is_usable_through_its_mock() {
        let mut statements = MockStatementRepository::new();
        statements
            .expect_list_unsent()
            .returning(|| Box::pin(async { Ok(Vec::new()) }));

        assert_eq!(count_unsent(&statements).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_event_log_reports_whether_the_event_is_new() {
        let id = ProviderEventId::parse("evt_1").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
        let mut log = MockProviderEventLog::new();
        log.expect_record()
            .with(eq(id.clone()), eq(now))
            .returning(|_, _| Box::pin(async { Ok(true) }));

        assert!(is_new_event(&log, &id, now).await.unwrap());
    }

    #[test]
    fn reading_an_event_is_synchronous_and_carries_the_billing_error() {
        let now = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
        let mut provider = MockPaymentProvider::new();
        provider
            .expect_read_event()
            .withf(|payload, _, _| payload == b"good")
            .returning(|_, _, _| {
                Ok(ProviderEvent {
                    id: ProviderEventId::parse("evt_1").unwrap(),
                    kind: ProviderEventKind::Ignored,
                })
            });
        provider
            .expect_read_event()
            .returning(|_, _, _| Err(BillingError::InvalidEventSignature));

        assert_eq!(
            read(&provider, b"good", now).unwrap().kind,
            ProviderEventKind::Ignored
        );
        assert!(matches!(
            read(&provider, b"forged", now),
            Err(CoreError::Billing(BillingError::InvalidEventSignature))
        ));
    }
}
