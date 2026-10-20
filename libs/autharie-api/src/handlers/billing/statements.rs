use autharie_auth::Identity;
use autharie_core::{
    CoreError, StatementView,
    billing::{BillingPeriod, Statement, StatementLine, StatementStatus, TierCode},
    organisation::OrganisationId,
};
use axum::{Extension, extract::State};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{errors::ApiError, response::Response, state::AppState};

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/billing/statements")]
pub struct StatementsRoute {
    pub organisation_id: Uuid,
}

#[derive(TypedPath, IntoParams, Deserialize)]
#[typed_path("/organisations/{organisation_id}/billing/statements/{period}")]
pub struct StatementRoute {
    pub organisation_id: Uuid,
    #[param(example = "2026-11")]
    pub period: String,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatementViewStatus {
    Estimate,
    Zero,
    Held,
    Draft,
    Sent,
    Paid,
    PaymentFailed,
    Uncollectible,
}

impl From<StatementStatus> for StatementViewStatus {
    fn from(status: StatementStatus) -> Self {
        match status {
            StatementStatus::Zero => Self::Zero,
            StatementStatus::Held => Self::Held,
            StatementStatus::Draft => Self::Draft,
            StatementStatus::Sent => Self::Sent,
            StatementStatus::Paid => Self::Paid,
            StatementStatus::PaymentFailed => Self::PaymentFailed,
            StatementStatus::Uncollectible => Self::Uncollectible,
        }
    }
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct StatementSummary {
    pub period: BillingPeriod,
    pub status: StatementStatus,
    pub total: i64,
    pub hosted_invoice_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct StatementLineResponse {
    pub deployment_id: Uuid,
    pub tier: TierCode,
    pub monthly_price: i64,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub covered_seconds: i64,
    pub amount: i64,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct StatementResponse {
    pub period: BillingPeriod,
    pub status: StatementViewStatus,
    pub total: i64,
    pub hosted_invoice_url: Option<String>,
    pub lines: Vec<StatementLineResponse>,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct ListStatementsResponse {
    pub data: Vec<StatementSummary>,
}

fn hosted_invoice_url(statement: &Statement) -> Option<String> {
    statement
        .invoice()
        .filter(|_| statement.status().is_unpaid())
        .map(|invoice| invoice.hosted_url.clone())
}

impl From<&StatementLine> for StatementLineResponse {
    fn from(line: &StatementLine) -> Self {
        Self {
            deployment_id: line.deployment_id.0,
            tier: line.tier.clone(),
            monthly_price: line.monthly_price.minor(),
            starts_at: line.starts_at,
            ends_at: line.ends_at,
            covered_seconds: line.covered_seconds,
            amount: line.amount.minor(),
        }
    }
}

impl From<&Statement> for StatementSummary {
    fn from(statement: &Statement) -> Self {
        Self {
            period: statement.period(),
            status: statement.status(),
            total: statement.total().minor(),
            hosted_invoice_url: hosted_invoice_url(statement),
        }
    }
}

impl From<&StatementView> for StatementResponse {
    fn from(view: &StatementView) -> Self {
        match view {
            StatementView::Estimate { period, lines } => Self {
                period: *period,
                status: StatementViewStatus::Estimate,
                total: lines.iter().map(|line| line.amount.minor()).sum(),
                hosted_invoice_url: None,
                lines: lines.iter().map(StatementLineResponse::from).collect(),
            },
            StatementView::Stored(statement) => Self {
                period: statement.period(),
                status: statement.status().into(),
                total: statement.total().minor(),
                hosted_invoice_url: hosted_invoice_url(statement),
                lines: statement
                    .lines()
                    .iter()
                    .map(StatementLineResponse::from)
                    .collect(),
            },
        }
    }
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/billing/statements",
    summary = "list the statements of an organisation",
    tag = "billing",
    description = "The closed months, newest first. The month in progress is not listed: read it \
                   by its period to get an estimate. `hosted_invoice_url` is given while the \
                   invoice is open to payment, and is null otherwise.",
    params(StatementsRoute),
    responses(
        (status = 200, description = "The statements", body = ListStatementsResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's billing", body = ApiError),
        (status = 404, description = "No such organisation", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_statements_handler(
    StatementsRoute { organisation_id }: StatementsRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<ListStatementsResponse>, ApiError> {
    let statements = state
        .service
        .statements(identity, OrganisationId(organisation_id))
        .await?;

    Ok(Response::OK(ListStatementsResponse {
        data: statements.iter().map(StatementSummary::from).collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/{organisation_id}/billing/statements/{period}",
    summary = "read the statement of one month",
    tag = "billing",
    description = "The period is `YYYY-MM`. For a closed month this is the statement as invoiced. \
                   For the month in progress nothing is stored yet, so the answer is an estimate \
                   of what has been used up to now, with the status `estimate`.",
    params(StatementRoute),
    responses(
        (status = 200, description = "The statement, or the estimate of the current month", body = StatementResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 403, description = "The caller may not view this organisation's billing", body = ApiError),
        (status = 404, description = "No such organisation, or no statement for this month", body = ApiError),
        (status = 422, description = "The period is not YYYY-MM", body = ApiError),
        (status = 500, description = "Internal Server Error", body = ApiError)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_statement_handler(
    StatementRoute {
        organisation_id,
        period,
    }: StatementRoute,
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response<StatementResponse>, ApiError> {
    let period = period.parse::<BillingPeriod>().map_err(CoreError::from)?;
    let now = Utc::now();
    let first_period = state.billing.as_deref().map_or_else(
        || BillingPeriod::containing(now),
        |runtime| runtime.first_period,
    );

    let view = state
        .service
        .statement(
            identity,
            OrganisationId(organisation_id),
            period,
            first_period,
            now,
        )
        .await?;

    Ok(Response::OK(StatementResponse::from(&view)))
}

#[cfg(test)]
mod tests {
    use autharie_core::{
        billing::{
            Money, ProviderInvoice, ProviderInvoiceId, StatementId, StatementParts, SubscriptionId,
        },
        deployments::DeploymentId,
        offers::Offer,
    };
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn period() -> BillingPeriod {
        BillingPeriod::new(2026, 11).expect("a period")
    }

    fn line(amount: i64) -> StatementLine {
        StatementLine {
            deployment_id: DeploymentId(Uuid::nil()),
            subscription_id: SubscriptionId::new(),
            tier: TierCode::legacy(Offer::Standard),
            monthly_price: Money::from_minor(4900),
            starts_at: period().starts_at(),
            ends_at: period().ends_at(),
            covered_seconds: period().seconds(),
            amount: Money::from_minor(amount),
        }
    }

    fn stored(status: StatementStatus) -> Statement {
        let at = Utc
            .with_ymd_and_hms(2026, 12, 1, 0, 0, 0)
            .single()
            .expect("a date");

        Statement::from(StatementParts {
            id: StatementId::new(),
            organisation_id: OrganisationId::new(),
            period: period(),
            lines: vec![line(4900), line(100)],
            status,
            invoice: Some(ProviderInvoice {
                id: ProviderInvoiceId::parse("in_secret").expect("an invoice"),
                hosted_url: "https://invoice.test/i/1".to_string(),
            }),
            created_at: at,
            updated_at: at,
        })
    }

    #[test]
    fn the_invoice_address_is_given_only_while_the_invoice_is_open_to_payment() {
        let cases = [
            (StatementStatus::Zero, false),
            (StatementStatus::Held, false),
            (StatementStatus::Draft, false),
            (StatementStatus::Sent, true),
            (StatementStatus::Paid, false),
            (StatementStatus::PaymentFailed, true),
            (StatementStatus::Uncollectible, true),
        ];

        for (status, shown) in cases {
            let statement = stored(status);

            assert_eq!(
                StatementSummary::from(&statement)
                    .hosted_invoice_url
                    .is_some(),
                shown,
                "{status}"
            );
            assert_eq!(
                StatementResponse::from(&StatementView::Stored(statement))
                    .hosted_invoice_url
                    .is_some(),
                shown,
                "{status}"
            );
        }
    }

    #[test]
    fn a_stored_statement_keeps_its_status_and_adds_up_its_lines() {
        let answer = StatementResponse::from(&StatementView::Stored(stored(StatementStatus::Sent)));

        assert_eq!(answer.status, StatementViewStatus::Sent);
        assert_eq!(answer.total, 5000);
        assert_eq!(answer.lines.len(), 2);
        assert_eq!(answer.period, period());
    }

    #[test]
    fn an_estimate_is_labelled_so_and_has_no_invoice() {
        let answer = StatementResponse::from(&StatementView::Estimate {
            period: period(),
            lines: vec![line(2000), line(500)],
        });

        assert_eq!(answer.status, StatementViewStatus::Estimate);
        assert_eq!(answer.total, 2500);
        assert_eq!(answer.hosted_invoice_url, None);
    }

    #[test]
    fn the_wire_shape_names_the_status_in_snake_case_and_hides_the_provider() {
        let answer = serde_json::to_value(StatementResponse::from(&StatementView::Stored(stored(
            StatementStatus::PaymentFailed,
        ))))
        .expect("a serializable answer");

        assert_eq!(answer["status"], "payment_failed");
        assert_eq!(answer["period"], "2026-11");
        assert_eq!(answer["hosted_invoice_url"], "https://invoice.test/i/1");
        assert_eq!(
            answer["lines"][0],
            json!({
                "deployment_id": Uuid::nil().to_string(),
                "tier": "legacy-standard",
                "monthly_price": 4900,
                "starts_at": "2026-11-01T00:00:00Z",
                "ends_at": "2026-12-01T00:00:00Z",
                "covered_seconds": 2_592_000,
                "amount": 4900
            })
        );
        assert!(!answer.to_string().contains("in_secret"));
    }

    #[test]
    fn a_summary_carries_the_stored_status() {
        let summary = StatementSummary::from(&stored(StatementStatus::Paid));

        assert_eq!(summary.status, StatementStatus::Paid);
        assert_eq!(summary.total, 5000);
        assert_eq!(summary.period, period());
    }
}
