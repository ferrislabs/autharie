use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    billing::{
        BillingError, BillingPeriod, Money,
        account::{BillingAccount, provider_reference},
        subscription::SubscriptionId,
        tier::TierCode,
    },
    deployments::DeploymentId,
    generate_uuid_v7,
    organisation::OrganisationId,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
pub struct StatementId(pub Uuid);

impl StatementId {
    pub fn new() -> Self {
        Self(generate_uuid_v7())
    }
}

impl Default for StatementId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for StatementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for StatementId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatementStatus {
    Zero,
    Held,
    Draft,
    Sent,
    Paid,
    PaymentFailed,
    Uncollectible,
}

impl StatementStatus {
    pub fn can_become(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Held, Self::Draft)
                | (Self::Draft, Self::Sent)
                | (
                    Self::Sent,
                    Self::Paid | Self::PaymentFailed | Self::Uncollectible
                )
                | (
                    Self::PaymentFailed,
                    Self::Paid | Self::PaymentFailed | Self::Uncollectible
                )
                | (Self::Uncollectible, Self::Paid)
        )
    }

    pub fn is_unsent(self) -> bool {
        matches!(self, Self::Held | Self::Draft)
    }

    pub fn is_unpaid(self) -> bool {
        matches!(self, Self::Sent | Self::PaymentFailed | Self::Uncollectible)
    }
}

impl fmt::Display for StatementStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => f.write_str("zero"),
            Self::Held => f.write_str("held"),
            Self::Draft => f.write_str("draft"),
            Self::Sent => f.write_str("sent"),
            Self::Paid => f.write_str("paid"),
            Self::PaymentFailed => f.write_str("payment_failed"),
            Self::Uncollectible => f.write_str("uncollectible"),
        }
    }
}

impl FromStr for StatementStatus {
    type Err = BillingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "zero" => Ok(Self::Zero),
            "held" => Ok(Self::Held),
            "draft" => Ok(Self::Draft),
            "sent" => Ok(Self::Sent),
            "paid" => Ok(Self::Paid),
            "payment_failed" => Ok(Self::PaymentFailed),
            "uncollectible" => Ok(Self::Uncollectible),
            _ => Err(BillingError::InvalidStatementStatus {
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct StatementLine {
    pub deployment_id: DeploymentId,
    pub subscription_id: SubscriptionId,
    pub tier: TierCode,
    pub monthly_price: Money,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub covered_seconds: i64,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderInvoiceId(String);

impl ProviderInvoiceId {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        provider_reference(raw).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInvoice {
    pub id: ProviderInvoiceId,
    pub hosted_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    id: StatementId,
    organisation_id: OrganisationId,
    period: BillingPeriod,
    lines: Vec<StatementLine>,
    status: StatementStatus,
    invoice: Option<ProviderInvoice>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementParts {
    pub id: StatementId,
    pub organisation_id: OrganisationId,
    pub period: BillingPeriod,
    pub lines: Vec<StatementLine>,
    pub status: StatementStatus,
    pub invoice: Option<ProviderInvoice>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<StatementParts> for Statement {
    fn from(parts: StatementParts) -> Self {
        Self {
            id: parts.id,
            organisation_id: parts.organisation_id,
            period: parts.period,
            lines: parts.lines,
            status: parts.status,
            invoice: parts.invoice,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }
}

impl Statement {
    pub fn compose(
        organisation_id: OrganisationId,
        period: BillingPeriod,
        lines: Vec<StatementLine>,
        account: Option<&BillingAccount>,
        at: DateTime<Utc>,
    ) -> Option<Self> {
        if lines.is_empty() {
            return None;
        }

        let total: Money = lines.iter().map(|line| line.amount).sum();

        let status = if total == Money::ZERO {
            StatementStatus::Zero
        } else if account.is_some_and(BillingAccount::can_be_invoiced) {
            StatementStatus::Draft
        } else {
            StatementStatus::Held
        };

        Some(Self {
            id: StatementId::new(),
            organisation_id,
            period,
            lines,
            status,
            invoice: None,
            created_at: at,
            updated_at: at,
        })
    }

    pub fn advance(
        &mut self,
        next: StatementStatus,
        at: DateTime<Utc>,
    ) -> Result<(), BillingError> {
        if !self.status.can_become(next) {
            return Err(BillingError::InvalidStatementTransition {
                id: self.id.0,
                from: self.status.to_string(),
                to: next.to_string(),
            });
        }

        self.status = next;
        self.updated_at = at;

        Ok(())
    }

    pub fn mark_sent(
        &mut self,
        invoice: ProviderInvoice,
        at: DateTime<Utc>,
    ) -> Result<(), BillingError> {
        self.advance(StatementStatus::Sent, at)?;
        self.invoice = Some(invoice);

        Ok(())
    }

    pub fn total(&self) -> Money {
        self.lines.iter().map(|line| line.amount).sum()
    }

    pub fn id(&self) -> StatementId {
        self.id
    }

    pub fn organisation_id(&self) -> OrganisationId {
        self.organisation_id
    }

    pub fn period(&self) -> BillingPeriod {
        self.period
    }

    pub fn lines(&self) -> &[StatementLine] {
        &self.lines
    }

    pub fn status(&self) -> StatementStatus {
        self.status
    }

    pub fn invoice(&self) -> Option<&ProviderInvoice> {
        self.invoice.as_ref()
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::billing::{
        BillingAccount, BillingAddress, BillingAddressInput, BillingDetails, BillingEmail,
        BillingName, PaymentMethodStatus, ProviderCustomerId,
    };

    const ALL: [StatementStatus; 7] = [
        StatementStatus::Zero,
        StatementStatus::Held,
        StatementStatus::Draft,
        StatementStatus::Sent,
        StatementStatus::Paid,
        StatementStatus::PaymentFailed,
        StatementStatus::Uncollectible,
    ];

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, day, 0, 0, 0).unwrap()
    }

    fn period() -> BillingPeriod {
        BillingPeriod::new(2026, 11).unwrap()
    }

    fn line(amount: i64) -> StatementLine {
        StatementLine {
            deployment_id: DeploymentId(Uuid::new_v4()),
            subscription_id: SubscriptionId::new(),
            tier: TierCode::legacy(crate::offers::Offer::Standard),
            monthly_price: Money::from_minor(4900),
            starts_at: period().starts_at(),
            ends_at: period().ends_at(),
            covered_seconds: period().seconds(),
            amount: Money::from_minor(amount),
        }
    }

    fn account(customer: bool, payment_method: PaymentMethodStatus) -> BillingAccount {
        let details = BillingDetails {
            name: BillingName::parse("Acme SAS").unwrap(),
            email: BillingEmail::parse("billing@acme.test").unwrap(),
            address: BillingAddress::parse(BillingAddressInput {
                line1: "1 rue de la Paix",
                line2: None,
                postal_code: "75002",
                city: "Paris",
                country: "FR",
            })
            .unwrap(),
            vat_number: None,
            siren: None,
        };

        BillingAccount {
            customer: customer.then(|| ProviderCustomerId::parse("cus_123").unwrap()),
            payment_method,
            ..BillingAccount::new(OrganisationId::new(), details)
        }
    }

    fn statement(status: StatementStatus) -> Statement {
        Statement::from(StatementParts {
            id: StatementId::new(),
            organisation_id: OrganisationId::new(),
            period: period(),
            lines: vec![line(4900)],
            status,
            invoice: None,
            created_at: at(1),
            updated_at: at(1),
        })
    }

    fn invoice() -> ProviderInvoice {
        ProviderInvoice {
            id: ProviderInvoiceId::parse("in_123").unwrap(),
            hosted_url: "https://pay.test/in_123".to_string(),
        }
    }

    #[test]
    fn the_transition_table_allows_exactly_the_documented_moves() {
        use StatementStatus::*;

        let allowed = [
            (Held, Draft),
            (Draft, Sent),
            (Sent, Paid),
            (Sent, PaymentFailed),
            (Sent, Uncollectible),
            (PaymentFailed, Paid),
            (PaymentFailed, PaymentFailed),
            (PaymentFailed, Uncollectible),
            (Uncollectible, Paid),
        ];

        for from in ALL {
            for to in ALL {
                assert_eq!(
                    from.can_become(to),
                    allowed.contains(&(from, to)),
                    "{from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn a_zero_statement_and_a_paid_statement_go_nowhere() {
        for to in ALL {
            assert!(!StatementStatus::Zero.can_become(to));
            assert!(!StatementStatus::Paid.can_become(to));
        }
    }

    #[test]
    fn unsent_and_unpaid_statuses_are_disjoint_and_exact() {
        use StatementStatus::*;

        for status in ALL {
            assert_eq!(status.is_unsent(), matches!(status, Held | Draft));
            assert_eq!(
                status.is_unpaid(),
                matches!(status, Sent | PaymentFailed | Uncollectible)
            );
            assert!(!(status.is_unsent() && status.is_unpaid()));
        }
    }

    #[test]
    fn a_status_round_trips_through_its_text() {
        for status in ALL {
            assert_eq!(status.to_string().parse::<StatementStatus>(), Ok(status));
        }

        assert_eq!(StatementStatus::PaymentFailed.to_string(), "payment_failed");
        assert_eq!(
            "settled".parse::<StatementStatus>(),
            Err(BillingError::InvalidStatementStatus {
                value: "settled".to_string()
            })
        );
    }

    #[test]
    fn a_provider_invoice_id_follows_the_provider_reference_rule() {
        assert_eq!(ProviderInvoiceId::parse(" in_1 ").unwrap().as_str(), "in_1");
        assert!(ProviderInvoiceId::parse("").is_err());
        assert!(ProviderInvoiceId::parse("in 1").is_err());
        assert!(ProviderInvoiceId::parse(&"i".repeat(256)).is_err());
    }

    #[test]
    fn composing_without_lines_gives_no_statement() {
        let account = account(true, PaymentMethodStatus::Valid);

        assert_eq!(
            Statement::compose(
                OrganisationId::new(),
                period(),
                Vec::new(),
                Some(&account),
                at(1)
            ),
            None
        );
    }

    #[test]
    fn a_statement_totalling_nothing_is_zero_whatever_the_account() {
        let statement = Statement::compose(
            OrganisationId::new(),
            period(),
            vec![line(0), line(0)],
            None,
            at(1),
        )
        .unwrap();

        assert_eq!(statement.status(), StatementStatus::Zero);
        assert_eq!(statement.total(), Money::ZERO);
    }

    #[test]
    fn a_statement_without_an_account_is_held() {
        let statement = Statement::compose(
            OrganisationId::new(),
            period(),
            vec![line(100)],
            None,
            at(1),
        )
        .unwrap();

        assert_eq!(statement.status(), StatementStatus::Held);
    }

    #[test]
    fn a_statement_whose_account_cannot_be_invoiced_is_held() {
        for account in [
            account(false, PaymentMethodStatus::Valid),
            account(true, PaymentMethodStatus::Missing),
        ] {
            let statement = Statement::compose(
                OrganisationId::new(),
                period(),
                vec![line(100)],
                Some(&account),
                at(1),
            )
            .unwrap();

            assert_eq!(statement.status(), StatementStatus::Held);
        }
    }

    #[test]
    fn a_statement_whose_account_can_be_invoiced_is_a_draft() {
        for payment_method in [PaymentMethodStatus::Valid, PaymentMethodStatus::Failing] {
            let account = account(true, payment_method);

            let statement = Statement::compose(
                OrganisationId::new(),
                period(),
                vec![line(100), line(250)],
                Some(&account),
                at(1),
            )
            .unwrap();

            assert_eq!(statement.status(), StatementStatus::Draft);
            assert_eq!(statement.total(), Money::from_minor(350));
        }
    }

    #[test]
    fn a_composed_statement_has_no_invoice_and_one_creation_instant() {
        let organisation_id = OrganisationId::new();

        let statement =
            Statement::compose(organisation_id, period(), vec![line(100)], None, at(1)).unwrap();

        assert_eq!(statement.organisation_id(), organisation_id);
        assert_eq!(statement.period(), period());
        assert_eq!(statement.lines().len(), 1);
        assert_eq!(statement.invoice(), None);
        assert_eq!(statement.created_at(), at(1));
        assert_eq!(statement.updated_at(), at(1));
    }

    #[test]
    fn advancing_along_the_table_updates_the_status_and_the_instant() {
        let mut statement = statement(StatementStatus::Held);

        statement.advance(StatementStatus::Draft, at(2)).unwrap();

        assert_eq!(statement.status(), StatementStatus::Draft);
        assert_eq!(statement.updated_at(), at(2));
        assert_eq!(statement.created_at(), at(1));
    }

    #[test]
    fn advancing_off_the_table_names_the_statement_and_both_statuses() {
        let mut statement = statement(StatementStatus::Paid);
        let id = statement.id();

        assert_eq!(
            statement.advance(StatementStatus::Draft, at(2)),
            Err(BillingError::InvalidStatementTransition {
                id: id.0,
                from: "paid".to_string(),
                to: "draft".to_string()
            })
        );
        assert_eq!(statement.status(), StatementStatus::Paid);
        assert_eq!(statement.updated_at(), at(1));
    }

    #[test]
    fn marking_a_draft_sent_stores_the_invoice() {
        let mut statement = statement(StatementStatus::Draft);

        statement.mark_sent(invoice(), at(3)).unwrap();

        assert_eq!(statement.status(), StatementStatus::Sent);
        assert_eq!(statement.invoice(), Some(&invoice()));
        assert_eq!(statement.updated_at(), at(3));
    }

    #[test]
    fn marking_anything_but_a_draft_sent_is_refused_and_stores_nothing() {
        for status in ALL.into_iter().filter(|s| *s != StatementStatus::Draft) {
            let mut statement = statement(status);

            assert!(matches!(
                statement.mark_sent(invoice(), at(3)),
                Err(BillingError::InvalidStatementTransition { .. })
            ));
            assert_eq!(statement.invoice(), None);
            assert_eq!(statement.status(), status);
        }
    }

    #[test]
    fn a_failed_payment_can_be_retried_into_a_payment() {
        let mut statement = statement(StatementStatus::Sent);

        statement
            .advance(StatementStatus::PaymentFailed, at(4))
            .unwrap();
        statement
            .advance(StatementStatus::PaymentFailed, at(5))
            .unwrap();
        statement.advance(StatementStatus::Paid, at(6)).unwrap();

        assert_eq!(statement.status(), StatementStatus::Paid);
    }
}
