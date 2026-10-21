use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::{Args, BillingArgs},
    billing::BillingRuntime,
    handlers::billing::billing_routes,
    state::AppState,
};
use autharie_core::{
    ArchiveConfig, AutharieService, DataPlaneConfig,
    backups::{BucketName, StoreEncryption},
    billing::BillingPeriod,
};
use autharie_stripe::{StripeConfig, StripePaymentProvider};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use chrono::Utc;
use clap::CommandFactory;
use futures::FutureExt;
use hmac::{Hmac, Mac};
use httpmock::{Mock, prelude::*};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgPool, Postgres, pool::PoolConnection, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const LOCK: i64 = 232_232;
const SECRET_KEY: &str = "sk_test_billing_events_0123456789";
const WEBHOOK_SECRET: &str = "whsec_billing_events_0123456789";
const ROUTE: &str = "/billing/provider-events";
const HELP_CHILD: &str = "AUTHARIE_RENDER_THE_HELP_AND_STOP";
const DOMAIN: &str = "tenants.billing-events.test";
const BUCKET: &str = "billing-events-archives";

fn signed_with(secret: &str, payload: &[u8]) -> String {
    let timestamp = Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("a key of any length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(payload);

    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

fn signed(payload: &[u8]) -> String {
    signed_with(WEBHOOK_SECRET, payload)
}

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

fn runtime(api_base: String) -> BillingRuntime {
    BillingRuntime {
        provider: StripePaymentProvider::new(StripeConfig {
            secret_key: SECRET_KEY.into(),
            webhook_secret: WEBHOOK_SECRET.into(),
            api_base,
        })
        .expect("a provider"),
        first_period: BillingPeriod::new(2031, 1).expect("a period"),
        return_url: "https://console.test/billing".to_string(),
    }
}

struct Installation {
    pool: PgPool,
    lock: PoolConnection<Postgres>,
    owner: Uuid,
    organisation: Uuid,
    data_plane: Uuid,
    statement: Uuid,
    tag: String,
    customer: String,
    invoice: String,
}

impl Installation {
    async fn seed(pool: PgPool, status: &str, method: &str) -> Self {
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
            data_plane: Uuid::new_v4(),
            statement: Uuid::new_v4(),
            customer: format!("cus_{tag}"),
            invoice: format!("in_{tag}"),
            tag,
        };

        sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, 'events', $3)")
            .bind(installation.owner)
            .bind(format!("{}@billing-events.test", installation.owner))
            .bind(installation.owner.to_string())
            .execute(&installation.pool)
            .await
            .expect("a user");
        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'Billing Events', $2, $3, $4, 'starter', 5, 2, 1, now(), now())",
        )
        .bind(installation.organisation)
        .bind(format!("be-{}", installation.organisation))
        .bind(installation.owner)
        .bind(status)
        .execute(&installation.pool)
        .await
        .expect("an organisation");
        sqlx::query(
            "INSERT INTO billing_accounts \
             (organisation_id, name, email, address_line1, postal_code, city, country, \
              provider_customer_id, payment_method) \
             VALUES ($1, 'Acme SAS', 'billing@acme.test', '1 rue de la Paix', '75002', \
                     'Paris', 'FR', $2, $3)",
        )
        .bind(installation.organisation)
        .bind(&installation.customer)
        .bind(method)
        .execute(&installation.pool)
        .await
        .expect("a billing account");
        sqlx::query(
            "INSERT INTO billing_statements \
             (id, organisation_id, period, status, total_minor, provider_invoice_id, \
              hosted_invoice_url) \
             VALUES ($1, $2, '2031-03', 'sent', 4900, $3, 'https://invoice.stripe.test/i/events')",
        )
        .bind(installation.statement)
        .bind(installation.organisation)
        .bind(&installation.invoice)
        .execute(&installation.pool)
        .await
        .expect("a sent statement");

        installation
    }

    fn event_id(&self, name: &str) -> String {
        format!("evt_{}_{name}", self.tag)
    }

    fn service(&self) -> AutharieService {
        AutharieService::with_config(
            self.pool.clone(),
            DataPlaneConfig {
                heartbeat_window: chrono::Duration::seconds(60),
                provisioning_timeout: chrono::Duration::minutes(10),
                deleted_retention: chrono::Duration::days(7),
            },
            ArchiveConfig {
                bucket: Some(BucketName::new(BUCKET).expect("a bucket")),
                encryption: StoreEncryption::Managed,
            },
        )
        .with_domain(Some(DOMAIN.to_string()))
    }

    fn state(&self, billing: Option<BillingRuntime>) -> AppState {
        AppState {
            args: Arc::new(Args::default()),
            service: self.service(),
            certificate_source: None,
            quickwit_search: None,
            quickwit_traces: None,
            billing: billing.map(Arc::new),
        }
    }

    fn app(&self, billing: Option<BillingRuntime>) -> Router {
        let state = self.state(billing);

        billing_routes(state.clone()).with_state(state)
    }

    fn invoice_event(&self, name: &str, kind: &str) -> Vec<u8> {
        event(
            &self.event_id(name),
            kind,
            json!({"id": self.invoice, "customer": self.customer}),
        )
    }

    async fn draft_statement(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO billing_statements (id, organisation_id, period, status, total_minor) \
             VALUES ($1, $2, '2031-04', 'draft', 4900)",
        )
        .bind(id)
        .bind(self.organisation)
        .execute(&self.pool)
        .await
        .expect("a draft statement");

        id
    }

    async fn record_invoice(&self, statement: Uuid, invoice: &str) {
        sqlx::query(
            "UPDATE billing_statements SET status = 'sent', provider_invoice_id = $2, \
             hosted_invoice_url = 'https://invoice.stripe.test/i/late' WHERE id = $1",
        )
        .bind(statement)
        .bind(invoice)
        .execute(&self.pool)
        .await
        .expect("an invoice recorded");
    }

    async fn status_of(&self, statement: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM billing_statements WHERE id = $1")
            .bind(statement)
            .fetch_one(&self.pool)
            .await
            .expect("the statement")
    }

    async fn statement_status(&self) -> String {
        sqlx::query_scalar("SELECT status FROM billing_statements WHERE id = $1")
            .bind(self.statement)
            .fetch_one(&self.pool)
            .await
            .expect("the statement")
    }

    async fn payment_method(&self) -> String {
        sqlx::query_scalar("SELECT payment_method FROM billing_accounts WHERE organisation_id = $1")
            .bind(self.organisation)
            .fetch_one(&self.pool)
            .await
            .expect("the account")
    }

    async fn set_payment_method(&self, method: &str) {
        sqlx::query("UPDATE billing_accounts SET payment_method = $2 WHERE organisation_id = $1")
            .bind(self.organisation)
            .bind(method)
            .execute(&self.pool)
            .await
            .expect("an account");
    }

    async fn organisation_status(&self) -> String {
        sqlx::query_scalar("SELECT status FROM organisations WHERE id = $1")
            .bind(self.organisation)
            .fetch_one(&self.pool)
            .await
            .expect("the organisation")
    }

    async fn events_recorded(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM billing_provider_events WHERE id LIKE $1")
            .bind(format!("evt_{}_%", self.tag))
            .fetch_one(&self.pool)
            .await
            .expect("the events")
    }

    async fn suspensions(&self) -> Vec<(bool, bool)> {
        sqlx::query_as(
            "SELECT suspended_organisation, lifted_at IS NULL FROM billing_suspensions \
             WHERE organisation_id = $1 ORDER BY suspended_at",
        )
        .bind(self.organisation)
        .fetch_all(&self.pool)
        .await
        .expect("the suspensions")
    }

    fn tier(&self, paid: bool) -> String {
        format!("ev-{}-{}", if paid { "paid" } else { "free" }, self.tag)
    }

    async fn fleet(&self) {
        sqlx::query(
            "INSERT INTO data_planes \
             (id, mode, region, status, capacity_cpu_millis, capacity_memory_mib, \
              capacity_storage_gib, created_at, updated_at) \
             VALUES ($1, 'shared', $2, 'active', 8000, 16384, 200, now(), now())",
        )
        .bind(self.data_plane)
        .bind(&self.tag)
        .execute(&self.pool)
        .await
        .expect("a data plane");
        for (paid, price, offer) in [(false, 0_i64, "free"), (true, 4900, "standard")] {
            sqlx::query(
                "INSERT INTO billing_tiers \
                 (code, name, monthly_price_minor, offer, features, engines, hostings, \
                  selectable) \
                 VALUES ($1, $1, $2, $3, '{mfa}', '{ferriskey}', '{shared}', false)",
            )
            .bind(self.tier(paid))
            .bind(price)
            .bind(offer)
            .execute(&self.pool)
            .await
            .expect("a tier");
        }
    }

    fn name_of(&self, label: &str) -> String {
        format!("{label}-{}", &self.tag[..8])
    }

    async fn deployment(&self, label: &str, status: &str, paid: bool) -> Uuid {
        let id = Uuid::new_v4();
        let name = self.name_of(label);
        sqlx::query(
            "INSERT INTO deployments \
             (id, organisation_id, dataplane_id, name, kind, status, namespace, version, \
              cpu_millis, memory_mib, storage_gib, created_by, created_at, updated_at, \
              hostname_slug) \
             VALUES ($1, $2, $3, $4, 'ferriskey', $5, $4, '26.0.0', \
                     500, 1024, 1, $6, now(), now(), $4)",
        )
        .bind(id)
        .bind(self.organisation)
        .bind(self.data_plane)
        .bind(&name)
        .bind(status)
        .bind(self.owner)
        .execute(&self.pool)
        .await
        .expect("a deployment");
        sqlx::query(
            "INSERT INTO billing_subscriptions \
             (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, \
              features, starts_at) \
             VALUES ($1, $2, $3, $4, $5, $6, '{mfa}', now() - interval '1 day')",
        )
        .bind(Uuid::new_v4())
        .bind(self.organisation)
        .bind(id)
        .bind(self.tier(paid))
        .bind(if paid { 4900_i64 } else { 0 })
        .bind(if paid { "standard" } else { "free" })
        .execute(&self.pool)
        .await
        .expect("a subscription");

        id
    }

    async fn back_up(&self, deployment: Uuid) {
        sqlx::query(
            "INSERT INTO backup_schedules \
             (deployment_id, organisation_id, cadence, zone, keep_last, keep_for_days, method) \
             VALUES ($1, $2, '{\"every\": \"daily\", \"at\": \"02:30:00\"}', 'UTC', 7, 30, \
                     'physical')",
        )
        .bind(deployment)
        .bind(self.organisation)
        .execute(&self.pool)
        .await
        .expect("a backup schedule");
    }

    async fn deployment_status(&self, deployment: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM deployments WHERE id = $1")
            .bind(deployment)
            .fetch_one(&self.pool)
            .await
            .expect("the deployment")
    }

    async fn set_deployment_status(&self, deployment: Uuid, status: &str) {
        sqlx::query("UPDATE deployments SET status = $2 WHERE id = $1")
            .bind(deployment)
            .bind(status)
            .execute(&self.pool)
            .await
            .expect("a deployment");
    }

    async fn actions_of(&self, deployment: Uuid) -> Vec<(String, Value)> {
        sqlx::query_as(
            "SELECT action_type, payload FROM actions WHERE deployment_id = $1 \
             ORDER BY created_at",
        )
        .bind(deployment)
        .fetch_all(&self.pool)
        .await
        .expect("the actions")
    }

    async fn subscription_is_open(&self, deployment: Uuid) -> bool {
        sqlx::query_scalar(
            "SELECT count(*) > 0 FROM billing_subscriptions \
             WHERE deployment_id = $1 AND ends_at IS NULL",
        )
        .bind(deployment)
        .fetch_one(&self.pool)
        .await
        .expect("the subscriptions")
    }

    async fn stopped_by_billing(&self) -> Vec<Uuid> {
        sqlx::query_scalar(
            "SELECT unnest(stopped_deployments) FROM billing_suspensions \
             WHERE organisation_id = $1",
        )
        .bind(self.organisation)
        .fetch_all(&self.pool)
        .await
        .expect("the stopped deployments")
    }

    async fn forget(mut self) {
        for statement in [
            "DELETE FROM billing_suspensions WHERE organisation_id = $1",
            "DELETE FROM billing_statements WHERE organisation_id = $1",
            "DELETE FROM billing_subscriptions WHERE organisation_id = $1",
            "DELETE FROM billing_accounts WHERE organisation_id = $1",
            "DELETE FROM deployments WHERE organisation_id = $1",
            "DELETE FROM organisations WHERE id = $1",
        ] {
            sqlx::query(statement)
                .bind(self.organisation)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("DELETE FROM data_planes WHERE id = $1")
            .bind(self.data_plane)
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM billing_tiers WHERE code LIKE $1")
            .bind(format!("ev-%-{}", self.tag))
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM billing_provider_events WHERE id LIKE $1")
            .bind(format!("evt_{}_%", self.tag))
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

fn event(id: &str, kind: &str, object: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": id,
        "object": "event",
        "type": kind,
        "data": {"object": object}
    }))
    .expect("an event")
}

async fn deliver(app: &Router, payload: &[u8], signature: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().method(Method::POST).uri(ROUTE);
    if let Some(signature) = signature {
        request = request.header("Stripe-Signature", signature);
    }

    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(payload.to_vec()))
                .expect("a request"),
        )
        .await
        .expect("a response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn deliver_signed(app: &Router, payload: &[u8]) -> (StatusCode, String) {
    deliver(app, payload, Some(&signed(payload))).await
}

async fn scenario<F>(status: &str, method: &str, body: F)
where
    F: AsyncFnOnce(&Installation),
{
    let Some(pool) = pool().await else {
        return;
    };
    let installation = Installation::seed(pool, status, method).await;

    let outcome = std::panic::AssertUnwindSafe(body(&installation))
        .catch_unwind()
        .await;
    installation.forget().await;

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

fn nowhere() -> String {
    "http://127.0.0.1:1".to_string()
}

#[tokio::test]
async fn a_paid_invoice_marks_the_statement_paid_and_the_method_valid() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let payload = installation.invoice_event("paid", "invoice.paid");

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            serde_json::from_str::<Value>(&body).expect("json"),
            json!({"received": true})
        );
        assert_eq!(installation.statement_status().await, "paid");
        assert_eq!(installation.payment_method().await, "valid");
        assert_eq!(installation.events_recorded().await, 1);
    })
    .await;
}

#[tokio::test]
async fn the_same_event_twice_is_applied_once() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let payload = installation.invoice_event("paid", "invoice.paid");

        let (first, _) = deliver_signed(&app, &payload).await;
        installation.set_payment_method("failing").await;
        let (second, body) = deliver_signed(&app, &payload).await;

        assert_eq!(first, StatusCode::OK);
        assert_eq!(second, StatusCode::OK, "{body}");
        assert_eq!(installation.events_recorded().await, 1);
        assert_eq!(installation.statement_status().await, "paid");
        assert_eq!(installation.payment_method().await, "failing");
    })
    .await;
}

#[tokio::test]
async fn a_forged_signature_is_refused_and_nothing_changes() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let payload = installation.invoice_event("forged", "invoice.paid");

        let forged = signed_with("whsec_somebody_else", &payload);
        let (status, body) = deliver(&app, &payload, Some(&forged)).await;
        let (missing, missing_body) = deliver(&app, &payload, None).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(missing, StatusCode::BAD_REQUEST, "{missing_body}");
        assert_eq!(installation.events_recorded().await, 0);
        assert_eq!(installation.statement_status().await, "sent");
        assert_eq!(installation.payment_method().await, "failing");
        for answer in [&body, &missing_body] {
            assert!(!answer.contains(WEBHOOK_SECRET), "{answer}");
            assert!(!answer.contains(SECRET_KEY), "{answer}");
        }
    })
    .await;
}

#[tokio::test]
async fn an_uncollectible_invoice_suspends_the_organisation_and_payment_lifts_it() {
    scenario("active", "valid", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));

        let (status, body) = deliver_signed(
            &app,
            &installation.invoice_event("lost", "invoice.marked_uncollectible"),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.statement_status().await, "uncollectible");
        assert_eq!(installation.organisation_status().await, "suspended");
        assert_eq!(installation.suspensions().await, vec![(true, true)]);

        let (status, body) =
            deliver_signed(&app, &installation.invoice_event("late", "invoice.paid")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.statement_status().await, "paid");
        assert_eq!(installation.organisation_status().await, "active");
        assert_eq!(installation.suspensions().await, vec![(true, false)]);
    })
    .await;
}

#[tokio::test]
async fn an_uncollectible_invoice_stops_the_paid_deployments_and_payment_starts_them_again() {
    scenario("active", "valid", async |installation| {
        installation.fleet().await;
        let paid = installation.deployment("paid", "successful", true).await;
        let free = installation.deployment("free", "successful", false).await;
        installation.back_up(paid).await;
        let app = installation.app(Some(runtime(nowhere())));

        let (status, body) = deliver_signed(
            &app,
            &installation.invoice_event("lost", "invoice.marked_uncollectible"),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.deployment_status(paid).await, "stopped");
        assert_eq!(installation.deployment_status(free).await, "successful");
        assert_eq!(installation.stopped_by_billing().await, vec![paid]);
        assert!(!installation.subscription_is_open(paid).await);
        assert!(installation.subscription_is_open(free).await);
        assert!(installation.actions_of(free).await.is_empty());
        let actions = installation.actions_of(paid).await;
        assert_eq!(actions.len(), 1, "{actions:?}");
        let (action_type, payload) = &actions[0];
        assert_eq!(action_type, "deployment.stop");
        assert_eq!(payload["stopped"], json!(true));
        assert_eq!(
            payload["hostname"],
            json!(format!("{}.{DOMAIN}", installation.name_of("paid")))
        );
        assert_eq!(payload["archive"]["encryption"], json!("AES256"));
        assert!(
            payload["archive"]["destination_path"]
                .as_str()
                .is_some_and(|path| path.starts_with(&format!("s3://{BUCKET}/"))),
            "{payload}"
        );

        let (status, body) =
            deliver_signed(&app, &installation.invoice_event("late", "invoice.paid")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.deployment_status(paid).await, "in_progress");
        assert_eq!(installation.deployment_status(free).await, "successful");
        assert!(installation.subscription_is_open(paid).await);
        assert!(installation.actions_of(free).await.is_empty());
        let actions = installation.actions_of(paid).await;
        let kinds: Vec<&str> = actions.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["deployment.stop", "deployment.start"],
            "{actions:?}"
        );
        let payload = &actions[1].1;
        assert_eq!(payload["stopped"], json!(false));
        assert_eq!(
            payload["hostname"],
            json!(format!("{}.{DOMAIN}", installation.name_of("paid")))
        );
        assert_eq!(payload["archive"]["encryption"], json!("AES256"));
    })
    .await;
}

#[tokio::test]
async fn payment_starts_only_the_deployments_billing_stopped() {
    scenario("active", "valid", async |installation| {
        installation.fleet().await;
        let billed = installation.deployment("billed", "successful", true).await;
        let parked = installation.deployment("parked", "stopped", true).await;
        let app = installation.app(Some(runtime(nowhere())));

        deliver_signed(
            &app,
            &installation.invoice_event("lost", "invoice.marked_uncollectible"),
        )
        .await;

        assert_eq!(installation.stopped_by_billing().await, vec![billed]);
        assert_eq!(installation.deployment_status(billed).await, "stopped");
        assert!(installation.subscription_is_open(parked).await);
        assert!(installation.actions_of(parked).await.is_empty());

        let (status, body) =
            deliver_signed(&app, &installation.invoice_event("late", "invoice.paid")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.deployment_status(billed).await, "in_progress");
        assert_eq!(installation.deployment_status(parked).await, "stopped");
        assert!(installation.actions_of(parked).await.is_empty());
        let started: Vec<String> = installation
            .actions_of(billed)
            .await
            .into_iter()
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(started, ["deployment.stop", "deployment.start"]);
    })
    .await;
}

#[tokio::test]
async fn a_deployment_billing_could_not_stop_is_stopped_by_the_billing_run_and_started_by_payment()
{
    scenario("active", "valid", async |installation| {
        installation.fleet().await;
        let running = installation.deployment("running", "successful", true).await;
        let upgrading = installation
            .deployment("upgrading", "upgrading", true)
            .await;
        let billing = runtime(nowhere());
        let app = installation.app(Some(runtime(nowhere())));

        deliver_signed(
            &app,
            &installation.invoice_event("lost", "invoice.marked_uncollectible"),
        )
        .await;

        assert_eq!(installation.deployment_status(running).await, "stopped");
        assert_eq!(installation.deployment_status(upgrading).await, "upgrading");
        assert!(installation.subscription_is_open(upgrading).await);
        assert!(installation.actions_of(upgrading).await.is_empty());
        assert_eq!(installation.stopped_by_billing().await, vec![running]);

        let run = installation
            .service()
            .run_billing(&billing.provider, billing.first_period, Utc::now())
            .await
            .expect("a billing run");

        assert_eq!((run.stopped, run.failed), (0, 0));
        assert_eq!(installation.deployment_status(upgrading).await, "upgrading");
        assert!(installation.actions_of(upgrading).await.is_empty());

        installation
            .set_deployment_status(upgrading, "successful")
            .await;
        let run = installation
            .service()
            .run_billing(&billing.provider, billing.first_period, Utc::now())
            .await
            .expect("a billing run");

        assert_eq!((run.stopped, run.failed), (1, 0));
        assert_eq!(installation.deployment_status(upgrading).await, "stopped");
        assert!(!installation.subscription_is_open(upgrading).await);
        assert_eq!(
            installation.stopped_by_billing().await,
            vec![running, upgrading]
        );
        let actions = installation.actions_of(upgrading).await;
        let kinds: Vec<&str> = actions.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(kinds, ["deployment.stop"], "{actions:?}");
        assert_eq!(actions[0].1["stopped"], json!(true));

        let run = installation
            .service()
            .run_billing(&billing.provider, billing.first_period, Utc::now())
            .await
            .expect("a billing run");

        assert_eq!((run.stopped, run.failed), (0, 0));
        assert_eq!(installation.actions_of(upgrading).await.len(), 1);

        let (status, body) =
            deliver_signed(&app, &installation.invoice_event("late", "invoice.paid")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        for deployment in [running, upgrading] {
            assert_eq!(
                installation.deployment_status(deployment).await,
                "in_progress"
            );
            assert!(installation.subscription_is_open(deployment).await);
            let actions = installation.actions_of(deployment).await;
            let kinds: Vec<&str> = actions.iter().map(|(kind, _)| kind.as_str()).collect();
            assert_eq!(
                kinds,
                ["deployment.stop", "deployment.start"],
                "{actions:?}"
            );
        }
        assert_eq!(installation.suspensions().await, vec![(true, false)]);
    })
    .await;
}

#[tokio::test]
async fn an_organisation_suspended_before_stays_suspended_after_payment() {
    scenario("suspended", "valid", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));

        deliver_signed(
            &app,
            &installation.invoice_event("lost", "invoice.marked_uncollectible"),
        )
        .await;

        assert_eq!(installation.suspensions().await, vec![(false, true)]);

        let (status, body) =
            deliver_signed(&app, &installation.invoice_event("late", "invoice.paid")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.statement_status().await, "paid");
        assert_eq!(installation.suspensions().await, vec![(false, false)]);
        assert_eq!(installation.organisation_status().await, "suspended");
    })
    .await;
}

#[tokio::test]
async fn a_failed_payment_is_recorded_and_flags_the_method() {
    scenario("active", "valid", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));

        let (status, body) = deliver_signed(
            &app,
            &installation.invoice_event("declined", "invoice.payment_failed"),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.statement_status().await, "payment_failed");
        assert_eq!(installation.payment_method().await, "failing");
        assert_eq!(installation.organisation_status().await, "active");
    })
    .await;
}

#[tokio::test]
async fn an_event_ahead_of_the_job_recording_its_invoice_is_retried_not_lost() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let draft = installation.draft_statement().await;
        let invoice = format!("in_{}_late", installation.tag);
        let payload = event(
            &installation.event_id("early"),
            "invoice.paid",
            json!({
                "id": invoice,
                "customer": installation.customer,
                "metadata": {"statement_id": draft.to_string()}
            }),
        );

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(body.contains("not recorded yet"), "{body}");
        assert_eq!(installation.events_recorded().await, 0);
        assert_eq!(installation.status_of(draft).await, "draft");
        assert_eq!(installation.payment_method().await, "failing");

        installation.record_invoice(draft, &invoice).await;
        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.events_recorded().await, 1);
        assert_eq!(installation.status_of(draft).await, "paid");
        assert_eq!(installation.payment_method().await, "valid");
    })
    .await;
}

#[tokio::test]
async fn an_invoice_of_nobody_here_is_acknowledged_and_remembered() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let draft = installation.draft_statement().await;
        let tag = &installation.tag;

        for (name, object) in [
            ("bare", json!({"id": format!("in_{tag}_a")})),
            (
                "unknown",
                json!({
                    "id": format!("in_{tag}_b"),
                    "metadata": {"statement_id": Uuid::new_v4().to_string()}
                }),
            ),
        ] {
            let payload = event(&installation.event_id(name), "invoice.paid", object);

            let (status, body) = deliver_signed(&app, &payload).await;

            assert_eq!(status, StatusCode::OK, "{name} {body}");
        }

        assert_eq!(installation.events_recorded().await, 2);
        assert_eq!(installation.status_of(draft).await, "draft");
        assert_eq!(installation.statement_status().await, "sent");
        assert_eq!(installation.payment_method().await, "failing");
    })
    .await;
}

#[tokio::test]
async fn a_signed_invoice_event_without_an_invoice_is_acknowledged_and_remembered() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let payload = event(
            &installation.event_id("anonymous"),
            "invoice.paid",
            json!({"object": "invoice"}),
        );

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.events_recorded().await, 1);
        assert_eq!(installation.statement_status().await, "sent");
    })
    .await;
}

#[tokio::test]
async fn an_event_nobody_cares_about_is_acknowledged_and_remembered() {
    scenario("active", "valid", async |installation| {
        let app = installation.app(Some(runtime(nowhere())));
        let payload = event(
            &installation.event_id("other"),
            "customer.updated",
            json!({"id": installation.customer}),
        );

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(installation.events_recorded().await, 1);
        assert_eq!(installation.statement_status().await, "sent");
    })
    .await;
}

#[tokio::test]
async fn a_payment_method_the_customer_added_becomes_the_default() {
    scenario("active", "missing", async |installation| {
        let server = MockServer::start();
        let default = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/customers/{}", installation.customer))
                .x_www_form_urlencoded_tuple("invoice_settings[default_payment_method]", "pm_77");
            then.status(200)
                .json_body(json!({"id": installation.customer}));
        });
        let app = installation.app(Some(runtime(server.base_url())));
        let payload = event(
            &installation.event_id("setup"),
            "setup_intent.succeeded",
            json!({"id": "seti_1", "customer": installation.customer, "payment_method": "pm_77"}),
        );

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        default.assert();
        assert_eq!(installation.payment_method().await, "valid");
        assert_eq!(installation.events_recorded().await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_failure_applying_an_event_records_nothing_so_the_provider_retries() {
    scenario("active", "missing", async |installation| {
        let server = MockServer::start();
        let path = format!("/v1/customers/{}", installation.customer);
        let mut refusing: Mock<'_> = server.mock(|when, then| {
            when.method(POST).path(path.clone());
            then.status(500)
                .json_body(json!({"error": {"type": "api_error"}}));
        });
        let app = installation.app(Some(runtime(server.base_url())));
        let payload = event(
            &installation.event_id("retry"),
            "setup_intent.succeeded",
            json!({"id": "seti_2", "customer": installation.customer, "payment_method": "pm_88"}),
        );

        let (status, body) = deliver_signed(&app, &payload).await;

        assert!(status.is_server_error(), "{status} {body}");
        assert_eq!(installation.events_recorded().await, 0);
        assert_eq!(installation.payment_method().await, "missing");
        assert!(!body.contains(SECRET_KEY), "{body}");

        refusing.delete();
        let accepting = server.mock(|when, then| {
            when.method(POST).path(path.clone());
            then.status(200)
                .json_body(json!({"id": installation.customer}));
        });

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        accepting.assert();
        assert_eq!(installation.events_recorded().await, 1);
        assert_eq!(installation.payment_method().await, "valid");
    })
    .await;
}

#[tokio::test]
async fn with_billing_disabled_the_endpoint_answers_503_and_writes_nothing() {
    scenario("active", "failing", async |installation| {
        let app = installation.app(None);
        let payload = installation.invoice_event("off", "invoice.paid");

        let (status, body) = deliver_signed(&app, &payload).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(body.contains("billing is not enabled on this installation"));
        assert!(!body.contains(WEBHOOK_SECRET), "{body}");
        assert!(!body.contains(SECRET_KEY), "{body}");
        assert_eq!(installation.events_recorded().await, 0);
        assert_eq!(installation.statement_status().await, "sent");
        assert_eq!(installation.payment_method().await, "failing");
    })
    .await;
}

#[test]
fn the_stripe_keys_appear_in_no_debug_output() {
    let args = BillingArgs {
        enabled: true,
        first_period: Some("2031-01".to_string()),
        return_url: Some("https://console.test/billing".to_string()),
        stripe_secret_key: Some(SECRET_KEY.to_string()),
        stripe_webhook_secret: Some(WEBHOOK_SECRET.to_string()),
        ..BillingArgs::default()
    };
    let runtime = BillingRuntime::from_args(&args)
        .expect("a runtime")
        .expect("billing is enabled");

    for output in [
        format!("{args:?}"),
        format!("{runtime:?}"),
        format!(
            "{:?}",
            Args {
                billing: args.clone(),
                ..Args::default()
            }
        ),
    ] {
        assert!(!output.contains(SECRET_KEY), "{output}");
        assert!(!output.contains(WEBHOOK_SECRET), "{output}");
    }
}

#[test]
fn the_stripe_secrets_never_appear_in_the_help_even_when_the_environment_holds_them() {
    if std::env::var_os(HELP_CHILD).is_some() {
        println!("{}", Args::command().render_long_help());
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "the_stripe_secrets_never_appear_in_the_help_even_when_the_environment_holds_them",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HELP_CHILD, "1")
        .env("STRIPE_SECRET_KEY", SECRET_KEY)
        .env("STRIPE_WEBHOOK_SECRET", WEBHOOK_SECRET)
        .output()
        .expect("the test binary runs");
    let help = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{help}");
    assert!(help.contains("--stripe-secret-key"), "{help}");
    assert!(help.contains("--stripe-webhook-secret"), "{help}");
    assert!(!help.contains(SECRET_KEY), "{help}");
    assert!(!help.contains(WEBHOOK_SECRET), "{help}");
}
