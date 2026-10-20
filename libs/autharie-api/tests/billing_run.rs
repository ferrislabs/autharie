use std::time::Duration;

use autharie_core::{AutharieService, billing::BillingPeriod};
use autharie_stripe::{StripeConfig, StripePaymentProvider};
use chrono::{DateTime, TimeZone, Utc};
use futures::FutureExt;
use httpmock::{Mock, prelude::*};
use serde_json::json;
use sqlx::{PgPool, Postgres, pool::PoolConnection, postgres::PgPoolOptions};
use uuid::Uuid;

const LOCK: i64 = 232_232;
const HOSTED_URL: &str = "https://invoice.stripe.test/i/billing-run";

type StatementRow = (String, Option<String>, Option<String>, i64);

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());

    let Some(url) = url else {
        assert!(
            std::env::var("REQUIRE_DATABASE_URL").is_err(),
            "REQUIRE_DATABASE_URL is set but DATABASE_URL is not"
        );
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };

    Some(
        PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but the database is unreachable"),
    )
}

fn march() -> BillingPeriod {
    BillingPeriod::new(2031, 3).expect("a period")
}

fn april() -> BillingPeriod {
    BillingPeriod::new(2031, 4).expect("a period")
}

fn at(month: u32, day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2031, month, day, hour, 0, 0)
        .single()
        .expect("an instant")
}

fn month_end() -> DateTime<Utc> {
    at(4, 2, 12)
}

struct Installation {
    pool: PgPool,
    lock: PoolConnection<Postgres>,
    owner: Uuid,
    organisation: Uuid,
    neighbour: Uuid,
    deployment: Uuid,
    codes: [String; 3],
}

impl Installation {
    async fn seed(pool: PgPool) -> Self {
        let mut lock = pool.acquire().await.expect("a connection");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(LOCK)
            .execute(&mut *lock)
            .await
            .expect("the billing lock");

        let tag = Uuid::new_v4().simple().to_string();
        let installation = Self {
            pool,
            lock,
            owner: Uuid::new_v4(),
            organisation: Uuid::new_v4(),
            neighbour: Uuid::new_v4(),
            deployment: Uuid::new_v4(),
            codes: [
                format!("run-free-{tag}"),
                format!("run-pro-{tag}"),
                format!("run-scale-{tag}"),
            ],
        };

        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'run', $3)")
            .bind(installation.owner)
            .bind(format!("{}@billing-run.test", installation.owner))
            .bind(installation.owner.to_string())
            .execute(&installation.pool)
            .await
            .expect("a user");
        for organisation in [installation.organisation, installation.neighbour] {
            sqlx::query(
                "INSERT INTO organisations \
                 (id, name, slug, owner_id, status, plan, max_instances, max_users, \
                  max_storage_gb, created_at, updated_at) \
                 VALUES ($1, 'run', $2, $3, 'active', 'starter', 5, 2, 1, now(), now())",
            )
            .bind(organisation)
            .bind(format!("br-{organisation}"))
            .bind(installation.owner)
            .execute(&installation.pool)
            .await
            .expect("an organisation");
        }
        for (code, price, offer) in [
            (&installation.codes[0], 0_i64, "free"),
            (&installation.codes[1], 4900, "standard"),
            (&installation.codes[2], 9900, "scale"),
        ] {
            sqlx::query(
                "INSERT INTO billing_tiers \
                 (code, name, monthly_price_minor, offer, features, engines, hostings, \
                  selectable) \
                 VALUES ($1, $1, $2, $3, '{mfa}', '{ferriskey}', '{shared}', false)",
            )
            .bind(code)
            .bind(price)
            .bind(offer)
            .execute(&installation.pool)
            .await
            .expect("a tier");
        }

        installation
    }

    fn service(&self) -> AutharieService {
        AutharieService::new(self.pool.clone())
    }

    async fn subscribe(
        &self,
        organisation: Uuid,
        deployment: Uuid,
        tier: usize,
        price: i64,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
    ) {
        sqlx::query(
            "INSERT INTO billing_subscriptions \
             (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, \
              features, starts_at, ends_at) \
             VALUES ($1, $2, $3, $4, $5, 'standard', '{mfa}', $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(organisation)
        .bind(deployment)
        .bind(&self.codes[tier])
        .bind(price)
        .bind(starts_at)
        .bind(ends_at)
        .execute(&self.pool)
        .await
        .expect("a subscription");
    }

    async fn upgrade_in_march(&self) {
        self.subscribe(
            self.organisation,
            self.deployment,
            1,
            4900,
            at(3, 1, 0),
            Some(at(3, 16, 0)),
        )
        .await;
        self.subscribe(
            self.organisation,
            self.deployment,
            2,
            9900,
            at(3, 16, 0),
            None,
        )
        .await;
    }

    async fn account(&self, organisation: Uuid, customer: Option<&str>, method: &str) {
        sqlx::query(
            "INSERT INTO billing_accounts \
             (organisation_id, name, email, address_line1, postal_code, city, country, \
              provider_customer_id, payment_method) \
             VALUES ($1, 'Acme SAS', 'billing@acme.test', '1 rue de la Paix', '75002', \
                     'Paris', 'FR', $2, $3)",
        )
        .bind(organisation)
        .bind(customer)
        .bind(method)
        .execute(&self.pool)
        .await
        .expect("a billing account");
    }

    async fn equip(&self, organisation: Uuid, customer: &str, method: &str) {
        sqlx::query(
            "UPDATE billing_accounts SET provider_customer_id = $2, payment_method = $3 \
             WHERE organisation_id = $1",
        )
        .bind(organisation)
        .bind(customer)
        .bind(method)
        .execute(&self.pool)
        .await
        .expect("an account to equip");
    }

    async fn statements_of(&self, organisation: Uuid) -> Vec<StatementRow> {
        sqlx::query_as(
            "SELECT s.status, s.provider_invoice_id, s.hosted_invoice_url, \
             (SELECT count(*) FROM billing_statement_lines l WHERE l.statement_id = s.id) \
             FROM billing_statements s WHERE s.organisation_id = $1 ORDER BY s.period",
        )
        .bind(organisation)
        .fetch_all(&self.pool)
        .await
        .expect("the statements")
    }

    async fn forget(mut self) {
        sqlx::query("DELETE FROM billing_statements WHERE organisation_id = ANY($1)")
            .bind([self.organisation, self.neighbour].as_slice())
            .execute(&self.pool)
            .await
            .expect("cleanup");
        for statement in [
            "DELETE FROM billing_subscriptions WHERE organisation_id = ANY($1)",
            "DELETE FROM billing_accounts WHERE organisation_id = ANY($1)",
            "DELETE FROM organisations WHERE id = ANY($1)",
        ] {
            sqlx::query(statement)
                .bind([self.organisation, self.neighbour].as_slice())
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("DELETE FROM billing_tiers WHERE code = ANY($1)")
            .bind(self.codes.as_slice())
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(self.owner)
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(LOCK)
            .execute(&mut *self.lock)
            .await
            .expect("the billing lock is released");
    }
}

struct Stripe<'a> {
    invoices: Mock<'a>,
    items: Mock<'a>,
    finalize: Mock<'a>,
}

impl<'a> Stripe<'a> {
    fn accepting(server: &'a MockServer, customer: &str, invoice: &str) -> Self {
        Self {
            invoices: server.mock(|when, then| {
                when.method(POST)
                    .path("/v1/invoices")
                    .x_www_form_urlencoded_tuple("customer", customer);
                then.status(200).json_body(json!({"id": invoice}));
            }),
            items: server.mock(|when, then| {
                when.method(POST)
                    .path("/v1/invoiceitems")
                    .x_www_form_urlencoded_tuple("customer", customer)
                    .x_www_form_urlencoded_tuple("invoice", invoice);
                then.status(200).json_body(json!({"id": "ii_1"}));
            }),
            finalize: server.mock(|when, then| {
                when.method(POST)
                    .path(format!("/v1/invoices/{invoice}/finalize"));
                then.status(200)
                    .json_body(json!({"id": invoice, "hosted_invoice_url": HOSTED_URL}));
            }),
        }
    }

    fn refusing(server: &'a MockServer, customer: &str) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(POST)
                .path("/v1/invoices")
                .x_www_form_urlencoded_tuple("customer", customer);
            then.status(500)
                .json_body(json!({"error": {"type": "api_error"}}));
        })
    }

    fn calls(&self) -> (usize, usize, usize) {
        (
            self.invoices.hits(),
            self.items.hits(),
            self.finalize.hits(),
        )
    }
}

fn provider(server: &MockServer) -> StripePaymentProvider {
    StripePaymentProvider::new(StripeConfig {
        secret_key: "sk_test_billing_run_0123456789".into(),
        webhook_secret: "whsec_billing_run_0123456789".into(),
        api_base: server.base_url(),
    })
    .expect("a provider")
}

fn invoice_id() -> String {
    format!("in_{}", Uuid::new_v4().simple())
}

fn customer_id() -> String {
    format!("cus_{}", Uuid::new_v4().simple())
}

async fn scenario<F>(body: F)
where
    F: AsyncFnOnce(&Installation),
{
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool).await;

    let outcome = std::panic::AssertUnwindSafe(body(&installation))
        .catch_unwind()
        .await;
    installation.forget().await;

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn after_month_end_an_organisation_gets_one_invoice_with_a_line_per_segment() {
    scenario(async |installation| {
        let customer = customer_id();
        let invoice = invoice_id();
        installation.upgrade_in_march().await;
        installation
            .account(installation.organisation, Some(&customer), "valid")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &customer, &invoice);

        let run = installation
            .service()
            .run_billing(&provider(&server), march(), month_end())
            .await
            .expect("a run");

        assert_eq!((run.closed, run.sent, run.failed), (1, 1, 0));
        assert_eq!(
            installation.statements_of(installation.organisation).await,
            vec![(
                "sent".to_string(),
                Some(invoice),
                Some(HOSTED_URL.to_string()),
                2
            )]
        );
        assert_eq!(stripe.calls(), (1, 2, 1));
    })
    .await;
}

#[tokio::test]
async fn running_again_sends_nothing() {
    scenario(async |installation| {
        let customer = customer_id();
        installation.upgrade_in_march().await;
        installation
            .account(installation.organisation, Some(&customer), "valid")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &customer, &invoice_id());
        let provider = provider(&server);
        let service = installation.service();

        service
            .run_billing(&provider, march(), month_end())
            .await
            .expect("a run");
        let again = service
            .run_billing(&provider, march(), month_end())
            .await
            .expect("a second run");

        assert_eq!((again.closed, again.sent, again.failed), (0, 0, 0));
        assert_eq!(stripe.calls(), (1, 2, 1));
        assert_eq!(
            installation
                .statements_of(installation.organisation)
                .await
                .len(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn free_subscriptions_alone_never_make_a_statement() {
    scenario(async |installation| {
        let customer = customer_id();
        installation
            .subscribe(
                installation.organisation,
                installation.deployment,
                0,
                0,
                at(3, 1, 0),
                None,
            )
            .await;
        installation
            .account(installation.organisation, Some(&customer), "valid")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &customer, &invoice_id());

        let run = installation
            .service()
            .run_billing(&provider(&server), march(), month_end())
            .await
            .expect("a run");

        assert_eq!((run.closed, run.sent, run.failed), (0, 0, 0));
        assert!(
            installation
                .statements_of(installation.organisation)
                .await
                .is_empty()
        );
        assert_eq!(stripe.calls(), (0, 0, 0));
    })
    .await;
}

#[tokio::test]
async fn nothing_before_the_first_period_is_billed() {
    scenario(async |installation| {
        let customer = customer_id();
        installation.upgrade_in_march().await;
        installation
            .account(installation.organisation, Some(&customer), "valid")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &customer, &invoice_id());

        let run = installation
            .service()
            .run_billing(&provider(&server), april(), month_end())
            .await
            .expect("a run");

        assert_eq!((run.closed, run.sent, run.failed), (0, 0, 0));
        assert!(
            installation
                .statements_of(installation.organisation)
                .await
                .is_empty()
        );
        assert_eq!(stripe.calls(), (0, 0, 0));
    })
    .await;
}

#[tokio::test]
async fn an_organisation_without_a_customer_is_held_until_it_has_one() {
    scenario(async |installation| {
        let customer = customer_id();
        let invoice = invoice_id();
        installation.upgrade_in_march().await;
        installation
            .account(installation.organisation, None, "missing")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &customer, &invoice);
        let provider = provider(&server);
        let service = installation.service();

        let held = service
            .run_billing(&provider, march(), month_end())
            .await
            .expect("a run");

        assert_eq!((held.closed, held.sent, held.failed), (1, 0, 0));
        assert_eq!(
            installation.statements_of(installation.organisation).await,
            vec![("held".to_string(), None, None, 2)]
        );
        assert_eq!(stripe.calls(), (0, 0, 0));

        installation
            .equip(installation.organisation, &customer, "valid")
            .await;
        let released = service
            .run_billing(&provider, march(), month_end())
            .await
            .expect("a second run");

        assert_eq!((released.closed, released.sent, released.failed), (0, 1, 0));
        assert_eq!(
            installation.statements_of(installation.organisation).await,
            vec![(
                "sent".to_string(),
                Some(invoice),
                Some(HOSTED_URL.to_string()),
                2
            )]
        );
        assert_eq!(stripe.calls(), (1, 2, 1));
    })
    .await;
}

#[tokio::test]
async fn a_failing_payment_method_does_not_hold_the_statement() {
    scenario(async |installation| {
        let customer = customer_id();
        installation.upgrade_in_march().await;
        installation
            .account(installation.organisation, Some(&customer), "failing")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &customer, &invoice_id());

        let run = installation
            .service()
            .run_billing(&provider(&server), march(), month_end())
            .await
            .expect("a run");

        assert_eq!((run.closed, run.sent, run.failed), (1, 1, 0));
        assert_eq!(stripe.calls(), (1, 2, 1));
        let statements = installation.statements_of(installation.organisation).await;
        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0].0, "sent");
    })
    .await;
}

#[tokio::test]
async fn a_statement_stripe_refuses_is_counted_and_the_others_still_go_out() {
    scenario(async |installation| {
        let good = customer_id();
        let bad = customer_id();
        installation.upgrade_in_march().await;
        installation
            .subscribe(
                installation.neighbour,
                Uuid::new_v4(),
                1,
                4900,
                at(3, 1, 0),
                None,
            )
            .await;
        installation
            .account(installation.organisation, Some(&good), "valid")
            .await;
        installation
            .account(installation.neighbour, Some(&bad), "valid")
            .await;
        let server = MockServer::start();
        let stripe = Stripe::accepting(&server, &good, &invoice_id());
        let refusal = Stripe::refusing(&server, &bad);

        let run = installation
            .service()
            .run_billing(&provider(&server), march(), month_end())
            .await
            .expect("a run that survives one refusal");

        assert_eq!((run.closed, run.sent, run.failed), (2, 1, 1));
        assert_eq!(refusal.hits(), 1);
        assert_eq!(stripe.calls(), (1, 2, 1));
        let sent = installation.statements_of(installation.organisation).await;
        assert_eq!(sent[0].0, "sent");
        let refused = installation.statements_of(installation.neighbour).await;
        assert_eq!(refused[0].0, "draft");
    })
    .await;
}
