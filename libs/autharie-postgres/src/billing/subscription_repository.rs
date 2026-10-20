use std::collections::BTreeSet;

use autharie_domain::{
    CoreError,
    billing::{
        BillingError, BillingPeriod, Money, Subscription, SubscriptionId, SubscriptionParts,
        SubscriptionTerms, TierCode, ports::SubscriptionRepository,
    },
    deployments::DeploymentId,
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
};
use autharie_macros::repository;
use autharie_persistence::SharedTx;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::columns::{failed, invalid, parsed};

#[cfg_attr(coverage_nightly, coverage(off))]
#[repository(domain = Subscription, backend = Postgres)]
pub struct PostgresSubscriptionRepository<'tx> {
    tx: SharedTx<'tx>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl<'tx> PostgresSubscriptionRepository<'tx> {
    pub fn new(tx: &SharedTx<'tx>) -> Self {
        Self { tx: tx.clone() }
    }
}

struct SubscriptionRow {
    id: Uuid,
    organisation_id: Uuid,
    deployment_id: Uuid,
    tier_code: String,
    monthly_price_minor: i64,
    offer: String,
    features: Vec<String>,
    starts_at: DateTime<Utc>,
    ends_at: Option<DateTime<Utc>>,
}

impl TryFrom<SubscriptionRow> for Subscription {
    type Error = CoreError;

    fn try_from(row: SubscriptionRow) -> Result<Self, Self::Error> {
        let features = row
            .features
            .iter()
            .map(|feature| parsed::<IamFeature>("billing_subscriptions.features", feature))
            .collect::<Result<BTreeSet<_>, _>>()?;

        let terms = SubscriptionTerms {
            tier: parsed::<TierCode>("billing_subscriptions.tier_code", &row.tier_code)?,
            monthly_price: Money::from_minor(row.monthly_price_minor),
            offer: parsed::<Offer>("billing_subscriptions.offer", &row.offer)?,
            features,
        };

        Subscription::try_from(SubscriptionParts {
            id: SubscriptionId(row.id),
            organisation_id: OrganisationId(row.organisation_id),
            deployment_id: DeploymentId(row.deployment_id),
            terms,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
        })
        .map_err(|reason| {
            invalid(
                "billing_subscriptions.ends_at",
                &format!("{:?}", row.ends_at),
                reason,
            )
        })
    }
}

impl SubscriptionRepository for PostgresSubscriptionRepository<'_> {
    async fn open(&self, subscription: &Subscription) -> Result<(), CoreError> {
        let terms = subscription.terms();
        let offer = terms.offer.to_string();
        let features: Vec<String> = terms.features.iter().map(ToString::to_string).collect();

        let mut tx = self.tx.lock().await;
        sqlx::query!(
            r#"
            INSERT INTO billing_subscriptions
                (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer,
                 features, starts_at, ends_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            subscription.id().0,
            subscription.organisation_id().0,
            subscription.deployment_id().0,
            terms.tier.as_str(),
            terms.monthly_price.minor(),
            offer,
            &features,
            subscription.starts_at(),
            subscription.ends_at()
        )
        .execute(&mut ***tx)
        .await
        .map_err(failed("open the subscription"))?;

        Ok(())
    }

    async fn close(&self, subscription: &Subscription) -> Result<(), CoreError> {
        let Some(ends_at) = subscription.ends_at() else {
            return Err(CoreError::InternalError(format!(
                "subscription {} is still open and cannot be stored as closed",
                subscription.id()
            )));
        };

        let closed = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                "UPDATE billing_subscriptions SET ends_at = $2 WHERE id = $1 AND ends_at IS NULL",
                subscription.id().0,
                ends_at
            )
            .execute(&mut ***tx)
            .await
        }
        .map_err(failed("close the subscription"))?;

        if closed.rows_affected() == 0 {
            return Err(CoreError::Billing(BillingError::SubscriptionClosed {
                id: subscription.id().0,
            }));
        }

        Ok(())
    }

    async fn latest_for(
        &self,
        deployment_id: &DeploymentId,
    ) -> Result<Option<Subscription>, CoreError> {
        let row = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SubscriptionRow,
                r#"
                SELECT id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer,
                       features, starts_at, ends_at
                FROM billing_subscriptions
                WHERE deployment_id = $1
                ORDER BY starts_at DESC, ends_at DESC NULLS FIRST, id DESC
                LIMIT 1
                "#,
                deployment_id.0
            )
            .fetch_optional(&mut ***tx)
            .await
        }
        .map_err(failed("find the latest subscription"))?;

        row.map(Subscription::try_from).transpose()
    }

    async fn list_in_period(
        &self,
        organisation_id: &OrganisationId,
        period: BillingPeriod,
    ) -> Result<Vec<Subscription>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SubscriptionRow,
                r#"
                SELECT id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer,
                       features, starts_at, ends_at
                FROM billing_subscriptions
                WHERE organisation_id = $1
                  AND validity && tstzrange($2, $3, '[)')
                ORDER BY deployment_id, starts_at
                "#,
                organisation_id.0,
                period.starts_at(),
                period.ends_at()
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("list the subscriptions of the period"))?;

        rows.into_iter().map(Subscription::try_from).collect()
    }

    async fn organisations_billable_in(
        &self,
        period: BillingPeriod,
    ) -> Result<Vec<OrganisationId>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query!(
                r#"
                SELECT DISTINCT organisation_id
                FROM billing_subscriptions
                WHERE monthly_price_minor > 0
                  AND validity && tstzrange($1, $2, '[)')
                ORDER BY organisation_id
                "#,
                period.starts_at(),
                period.ends_at()
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("list the organisations billable in the period"))?;

        Ok(rows
            .into_iter()
            .map(|row| OrganisationId(row.organisation_id))
            .collect())
    }

    async fn list_open_billable(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Vec<Subscription>, CoreError> {
        let rows = {
            let mut tx = self.tx.lock().await;
            sqlx::query_as!(
                SubscriptionRow,
                r#"
                SELECT id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer,
                       features, starts_at, ends_at
                FROM billing_subscriptions
                WHERE organisation_id = $1
                  AND ends_at IS NULL
                  AND monthly_price_minor > 0
                ORDER BY deployment_id, starts_at
                "#,
                organisation_id.0
            )
            .fetch_all(&mut ***tx)
            .await
        }
        .map_err(failed("list the open billable subscriptions"))?;

        rows.into_iter().map(Subscription::try_from).collect()
    }
}
