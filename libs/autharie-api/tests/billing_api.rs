use std::{sync::Arc, time::Duration};

use autharie_api::{
    args::Args,
    billing::BillingRuntime,
    handlers::{
        billing::{
            account::{get_billing_account_handler, put_billing_account_handler},
            payment_method_session::payment_method_session_handler,
            statements::{get_statement_handler, list_statements_handler},
            tiers::list_tiers_handler,
        },
        deployments::{
            create_deployment::create_deployment_handler,
            features::get_deployment_features_handler,
            subscription::{get_subscription_handler, put_subscription_handler},
        },
    },
    state::AppState,
};
use autharie_auth::{Identity, User};
use autharie_core::{
    AutharieService,
    billing::BillingPeriod,
    dataplane::{
        ports::DataPlaneService,
        value_objects::{Capacity, CreateDataplaneCommand, DataPlaneAllocation, Region},
    },
};
use autharie_stripe::{StripeConfig, StripePaymentProvider};
use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use axum_extra::routing::RouterExt;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use futures::FutureExt;
use httpmock::{Method::POST, MockServer};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, pool::PoolConnection, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const LOCK: i64 = 232_232;
const SECRET_KEY: &str = "sk_test_billing_api_0123456789";
const WEBHOOK_SECRET: &str = "whsec_billing_api_0123456789";
const ZONE: &str = "tenants.autharie.test";
const GATEWAY: &str = "203.0.113.7";
const VIEW_INSTANCES: i64 = 1 << 2;
const VIEW_BILLING: i64 = 1 << 12;
const MANAGE_BILLING: i64 = 1 << 13;
const ALL_FEATURES: [&str; 8] = [
    "sso_connectors",
    "mfa",
    "directory_federation",
    "custom_domain",
    "branding",
    "analytics",
    "compliance",
    "delegated_admin",
];

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

fn runtime(api_base: String, first_period: BillingPeriod) -> BillingRuntime {
    BillingRuntime {
        provider: StripePaymentProvider::new(StripeConfig {
            secret_key: SECRET_KEY.into(),
            webhook_secret: WEBHOOK_SECRET.into(),
            api_base,
        })
        .expect("a provider"),
        first_period,
        return_url: "https://console.test/billing".to_string(),
    }
}

fn far_first_period() -> BillingPeriod {
    BillingPeriod::new(2031, 1).expect("a period")
}

fn identity(sub: Uuid) -> Identity {
    Identity::User(User {
        id: sub.to_string(),
        username: "billing-api".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

fn start_of_this_month() -> DateTime<Utc> {
    let today = Utc::now();
    let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .expect("the first of the month");

    Utc.from_utc_datetime(&first)
}

fn this_period() -> String {
    Utc::now().format("%Y-%m").to_string()
}

fn account_body(name: &str) -> Value {
    json!({
        "name": name,
        "email": "billing@acme.test",
        "address": {
            "line1": "1 rue de la Paix",
            "line2": null,
            "postal_code": "75002",
            "city": "Paris",
            "country": "fr"
        },
        "vat_number": "FR12345678901",
        "siren": "123456789"
    })
}

struct People {
    operator: Uuid,
    owner: Uuid,
    reader: Uuid,
    manager: Uuid,
    watcher: Uuid,
    nobody: Uuid,
}

struct Installation {
    pool: PgPool,
    lock: PoolConnection<Postgres>,
    tag: String,
    people: People,
    organisation: Uuid,
    region: String,
    release: String,
    tiers: Vec<String>,
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
            people: People {
                operator: Uuid::new_v4(),
                owner: Uuid::new_v4(),
                reader: Uuid::new_v4(),
                manager: Uuid::new_v4(),
                watcher: Uuid::new_v4(),
                nobody: Uuid::new_v4(),
            },
            organisation: Uuid::new_v4(),
            region: format!("billing-api-{tag}"),
            release: format!("0.7.{}", Uuid::new_v4().as_u128() as u32 % 1_000_000),
            tiers: Vec::new(),
            tag,
        };

        for (id, label) in [
            (installation.people.operator, "operator"),
            (installation.people.owner, "owner"),
            (installation.people.reader, "reader"),
            (installation.people.manager, "manager"),
            (installation.people.watcher, "watcher"),
            (installation.people.nobody, "nobody"),
        ] {
            sqlx::query("INSERT INTO users (id, email, name, sub) VALUES ($1, $2, $3, $4)")
                .bind(id)
                .bind(format!("{id}@billing-api.test"))
                .bind(label)
                .bind(id.to_string())
                .execute(&installation.pool)
                .await
                .expect("a user");
        }

        sqlx::query(
            "INSERT INTO organisations \
             (id, name, slug, owner_id, status, plan, max_instances, max_users, \
              max_storage_gb, created_at, updated_at) \
             VALUES ($1, 'Billing Api', $2, $3, 'active', 'starter', 5, 10, 1, now(), now())",
        )
        .bind(installation.organisation)
        .bind(format!("ba-{}", installation.organisation))
        .bind(installation.people.owner)
        .execute(&installation.pool)
        .await
        .expect("an organisation");

        installation
            .grant(installation.people.reader, VIEW_BILLING)
            .await;
        installation
            .grant(
                installation.people.manager,
                VIEW_BILLING | MANAGE_BILLING | VIEW_INSTANCES,
            )
            .await;
        installation
            .grant(installation.people.watcher, VIEW_INSTANCES)
            .await;
        installation.grant(installation.people.nobody, 0).await;

        sqlx::query(
            "INSERT INTO releases (kind, version, status, risk) \
             VALUES ('ferriskey', $1, 'available', 'none')",
        )
        .bind(&installation.release)
        .execute(&installation.pool)
        .await
        .expect("a release");

        installation
    }

    async fn grant(&self, user: Uuid, permissions: i64) {
        let member = Uuid::new_v4();
        sqlx::query("INSERT INTO members (id, user_id, organisation_id) VALUES ($1, $2, $3)")
            .bind(member)
            .bind(user)
            .bind(self.organisation)
            .execute(&self.pool)
            .await
            .expect("a member");

        if permissions == 0 {
            return;
        }

        let role = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO roles (id, name, permissions, organisation_id) VALUES ($1, $2, $3, $4)",
        )
        .bind(role)
        .bind(format!("role-{role}"))
        .bind(permissions)
        .bind(self.organisation)
        .execute(&self.pool)
        .await
        .expect("a role");
        sqlx::query("INSERT INTO member_roles (member_id, role_id) VALUES ($1, $2)")
            .bind(member)
            .bind(role)
            .execute(&self.pool)
            .await
            .expect("a grant");
    }

    fn service(&self) -> AutharieService {
        AutharieService::new(self.pool.clone())
            .with_domain(Some(ZONE.to_string()))
            .with_platform_labels(vec!["control".to_string()])
    }

    fn app(&self, user: Uuid, billing: Option<BillingRuntime>) -> Router {
        let mut args = Args::default();
        args.dataplane.default_region = self.region.clone();

        Router::new()
            .typed_get(list_tiers_handler)
            .typed_get(get_billing_account_handler)
            .typed_put(put_billing_account_handler)
            .typed_post(payment_method_session_handler)
            .typed_get(list_statements_handler)
            .typed_get(get_statement_handler)
            .typed_get(get_subscription_handler)
            .typed_put(put_subscription_handler)
            .typed_get(get_deployment_features_handler)
            .typed_post(create_deployment_handler)
            .layer(Extension(identity(user)))
            .with_state(AppState {
                args: Arc::new(args),
                service: self.service(),
                certificate_source: None,
                quickwit_search: None,
                quickwit_traces: None,
                billing: billing.map(Arc::new),
            })
    }

    fn billing(&self, suffix: &str) -> String {
        format!("/organisations/{}/billing/{suffix}", self.organisation)
    }

    fn deployment(&self, deployment: &str, suffix: &str) -> String {
        format!(
            "/organisations/{}/deployments/{deployment}/{suffix}",
            self.organisation
        )
    }

    async fn serve_region(&self) {
        let service = self.service();
        service
            .ensure_first_operator(&self.people.operator.to_string())
            .await
            .expect("the operator holds every right");
        let registered = service
            .create_dataplane(
                identity(self.people.operator),
                CreateDataplaneCommand {
                    region: Region::new(self.region.clone()),
                    allocation: DataPlaneAllocation::Shared,
                    capacity: Capacity::new(64_000, 131_072, 2_000).expect("a capacity"),
                },
            )
            .await
            .expect("a shared data plane");
        sqlx::query(
            "UPDATE data_planes SET status = 'active', last_seen_at = now(), \
             herald_client_id = $2, herald_subject = $3, gateway_address = $4 WHERE id = $1",
        )
        .bind(registered.dataplane.id.0)
        .bind(format!("herald-{}", registered.dataplane.id.0))
        .bind(Uuid::new_v4().to_string())
        .bind(GATEWAY)
        .execute(&self.pool)
        .await
        .expect("a data plane that reports");
    }

    async fn deploy(&self, name: &str, tier: Option<&str>) -> (StatusCode, Value) {
        let mut body = json!({
            "name": name,
            "kind": "ferriskey",
            "version": self.release,
            "environment": "production",
            "offer": "standard",
        });
        if let Some(tier) = tier {
            body["tier"] = json!(tier);
        }

        let (status, text) = call(
            &self.app(self.people.owner, None),
            Method::POST,
            &format!("/organisations/{}/deployments", self.organisation),
            Some(body),
        )
        .await;

        (status, json_of(&text))
    }

    async fn account(&self, name: &str, customer: Option<&str>, method: &str) {
        sqlx::query(
            "INSERT INTO billing_accounts \
             (organisation_id, name, email, address_line1, postal_code, city, country, \
              provider_customer_id, payment_method) \
             VALUES ($1, $2, 'billing@acme.test', '1 rue de la Paix', '75002', \
                     'Paris', 'FR', $3, $4)",
        )
        .bind(self.organisation)
        .bind(name)
        .bind(customer)
        .bind(method)
        .execute(&self.pool)
        .await
        .expect("a billing account");
    }

    async fn paid_tier(&mut self, price: i64) -> String {
        let code = format!("ba-{}-{}", self.tag, self.tiers.len());
        sqlx::query(
            "INSERT INTO billing_tiers \
             (code, name, monthly_price_minor, offer, features, engines, hostings, selectable) \
             VALUES ($1, $1, $2, 'standard', '{mfa}', '{ferriskey}', '{shared}', false)",
        )
        .bind(&code)
        .bind(price)
        .execute(&self.pool)
        .await
        .expect("a tier");
        self.tiers.push(code.clone());

        code
    }

    async fn subscribe(
        &self,
        deployment: Uuid,
        tier: &str,
        price: i64,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO billing_subscriptions \
             (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, \
              features, starts_at, ends_at) \
             VALUES ($1, $2, $3, $4, $5, 'standard', '{mfa}', $6, $7)",
        )
        .bind(id)
        .bind(self.organisation)
        .bind(deployment)
        .bind(tier)
        .bind(price)
        .bind(starts_at)
        .bind(ends_at)
        .execute(&self.pool)
        .await
        .expect("a subscription");

        id
    }

    async fn statement(
        &mut self,
        period: &str,
        status: &str,
        invoice: Option<(&str, &str)>,
        amount: i64,
    ) {
        let tier = self.paid_tier(amount).await;
        let (year, month) = period.split_once('-').expect("a period");
        let starts_at = Utc
            .with_ymd_and_hms(
                year.parse().expect("a year"),
                month.parse().expect("a month"),
                1,
                0,
                0,
                0,
            )
            .single()
            .expect("a date");
        let ends_at = starts_at + chrono::Duration::days(28);
        let deployment = Uuid::new_v4();
        let subscription = self
            .subscribe(deployment, &tier, amount, starts_at, Some(ends_at))
            .await;
        let statement = Uuid::new_v4();
        let (invoice_id, hosted_url) = invoice.unzip();

        sqlx::query(
            "INSERT INTO billing_statements \
             (id, organisation_id, period, status, total_minor, provider_invoice_id, \
              hosted_invoice_url) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(statement)
        .bind(self.organisation)
        .bind(period)
        .bind(status)
        .bind(amount)
        .bind(invoice_id)
        .bind(hosted_url)
        .execute(&self.pool)
        .await
        .expect("a statement");
        sqlx::query(
            "INSERT INTO billing_statement_lines \
             (statement_id, position, deployment_id, subscription_id, tier_code, \
              monthly_price_minor, starts_at, ends_at, covered_seconds, amount_minor) \
             VALUES ($1, 0, $2, $3, $4, $5, $6, $7, $8, $5)",
        )
        .bind(statement)
        .bind(deployment)
        .bind(subscription)
        .bind(&tier)
        .bind(amount)
        .bind(starts_at)
        .bind(ends_at)
        .bind(28_i64 * 86_400)
        .execute(&self.pool)
        .await
        .expect("a statement line");
    }

    async fn stored_account(&self) -> Option<(String, Option<String>, String)> {
        sqlx::query_as(
            "SELECT name, provider_customer_id, payment_method FROM billing_accounts \
             WHERE organisation_id = $1",
        )
        .bind(self.organisation)
        .fetch_optional(&self.pool)
        .await
        .expect("the account")
    }

    async fn subscriptions_of(&self, deployment: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM billing_subscriptions WHERE deployment_id = $1")
            .bind(deployment)
            .fetch_one(&self.pool)
            .await
            .expect("the subscriptions")
    }

    async fn tier_of(&self, deployment: &str) -> String {
        sqlx::query_scalar(
            "SELECT tier_code FROM billing_subscriptions WHERE deployment_id = $1 \
             ORDER BY starts_at DESC LIMIT 1",
        )
        .bind(deployment.parse::<Uuid>().expect("a uuid"))
        .fetch_one(&self.pool)
        .await
        .expect("the subscription")
    }

    async fn deployments_stored(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM deployments WHERE organisation_id = $1")
            .bind(self.organisation)
            .fetch_one(&self.pool)
            .await
            .expect("the deployments")
    }

    async fn statements_stored(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM billing_statements WHERE organisation_id = $1")
            .bind(self.organisation)
            .fetch_one(&self.pool)
            .await
            .expect("the statements")
    }

    async fn forget(mut self) {
        for statement in [
            "DELETE FROM billing_statements WHERE organisation_id = $1",
            "DELETE FROM actions WHERE deployment_id IN \
             (SELECT id FROM deployments WHERE organisation_id = $1)",
            "DELETE FROM backup_schedules WHERE deployment_id IN \
             (SELECT id FROM deployments WHERE organisation_id = $1)",
            "DELETE FROM deployments WHERE organisation_id = $1",
            "DELETE FROM billing_subscriptions WHERE organisation_id = $1",
            "DELETE FROM billing_accounts WHERE organisation_id = $1",
            "DELETE FROM organisations WHERE id = $1",
        ] {
            sqlx::query(statement)
                .bind(self.organisation)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("DELETE FROM billing_tiers WHERE code = ANY($1)")
            .bind(self.tiers.as_slice())
            .execute(&self.pool)
            .await
            .expect("cleanup");
        for statement in [
            "DELETE FROM actions WHERE dataplane_id IN \
             (SELECT id FROM data_planes WHERE region LIKE $1 || '%')",
            "DELETE FROM data_planes WHERE region LIKE $1 || '%'",
        ] {
            sqlx::query(statement)
                .bind(&self.region)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("DELETE FROM releases WHERE kind = 'ferriskey' AND version = $1")
            .bind(&self.release)
            .execute(&self.pool)
            .await
            .expect("cleanup");
        sqlx::query("DELETE FROM platform_operators WHERE subject = $1")
            .bind(self.people.operator.to_string())
            .execute(&self.pool)
            .await
            .expect("cleanup");
        for id in [
            self.people.operator,
            self.people.owner,
            self.people.reader,
            self.people.manager,
            self.people.watcher,
            self.people.nobody,
        ] {
            sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .expect("cleanup");
        }
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(LOCK)
            .execute(&mut *self.lock)
            .await
            .expect("the billing lock is released");
    }
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };

    let response = app
        .clone()
        .oneshot(request.body(body).expect("a request"))
        .await
        .expect("a response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    call(app, Method::GET, uri, None).await
}

async fn scenario<F>(body: F)
where
    F: AsyncFnOnce(&mut Installation),
{
    let Some(pool) = pool().await else {
        return;
    };
    let mut installation = Installation::seed(pool).await;

    let outcome = std::panic::AssertUnwindSafe(body(&mut installation))
        .catch_unwind()
        .await;
    installation.forget().await;

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn only_a_member_who_may_view_billing_reads_the_account() {
    scenario(async |installation| {
        installation.account("Acme SAS", None, "missing").await;
        let path = installation.billing("account");

        for refused in [installation.people.nobody, installation.people.watcher] {
            let (status, body) = get(&installation.app(refused, None), &path).await;

            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert!(!body.contains("Acme SAS"), "{body}");
        }

        for allowed in [installation.people.reader, installation.people.owner] {
            let (status, body) = get(&installation.app(allowed, None), &path).await;

            assert_eq!(status, StatusCode::OK, "{body}");
            let account = json_of(&body);
            assert_eq!(account["name"], "Acme SAS");
            assert_eq!(account["email"], "billing@acme.test");
            assert_eq!(account["address"]["line1"], "1 rue de la Paix");
            assert_eq!(account["address"]["line2"], Value::Null);
            assert_eq!(account["address"]["postal_code"], "75002");
            assert_eq!(account["address"]["city"], "Paris");
            assert_eq!(account["address"]["country"], "FR");
            assert_eq!(account["vat_number"], Value::Null);
            assert_eq!(account["siren"], Value::Null);
            assert_eq!(account["payment_method"], "missing");
            assert_eq!(account["complete"], false);
        }
    })
    .await;
}

#[tokio::test]
async fn an_organisation_without_a_billing_account_answers_404() {
    scenario(async |installation| {
        let (status, body) = get(
            &installation.app(installation.people.reader, None),
            &installation.billing("account"),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    })
    .await;
}

#[tokio::test]
async fn a_member_who_may_not_manage_billing_changes_nothing() {
    scenario(async |installation| {
        installation
            .account("Acme SAS", Some("cus_untouched"), "valid")
            .await;
        let server = MockServer::start();
        let provider = server.mock(|when, then| {
            when.method(POST);
            then.status(200).json_body(json!({"id": "cus_other"}));
        });
        let deployment = Uuid::new_v4();

        for refused in [
            installation.people.reader,
            installation.people.watcher,
            installation.people.nobody,
        ] {
            let app = installation.app(
                refused,
                Some(runtime(server.base_url(), far_first_period())),
            );

            let (put, put_body) = call(
                &app,
                Method::PUT,
                &installation.billing("account"),
                Some(account_body("Changed")),
            )
            .await;
            let (session, session_body) = call(
                &app,
                Method::POST,
                &installation.billing("payment-method-session"),
                None,
            )
            .await;
            let (subscription, subscription_body) = call(
                &app,
                Method::PUT,
                &installation.deployment(&deployment.to_string(), "subscription"),
                Some(json!({"tier": "standard-2026"})),
            )
            .await;

            assert_eq!(put, StatusCode::FORBIDDEN, "{put_body}");
            assert_eq!(session, StatusCode::FORBIDDEN, "{session_body}");
            assert_eq!(subscription, StatusCode::FORBIDDEN, "{subscription_body}");
        }

        assert_eq!(
            installation.stored_account().await,
            Some((
                "Acme SAS".to_string(),
                Some("cus_untouched".to_string()),
                "valid".to_string()
            ))
        );
        assert_eq!(installation.subscriptions_of(deployment).await, 0);
        provider.assert_hits(0);
    })
    .await;
}

#[tokio::test]
async fn the_current_month_is_an_estimate_of_what_has_been_used_so_far() {
    scenario(async |installation| {
        let tier = installation.paid_tier(99_900).await;
        let deployment = Uuid::new_v4();
        installation
            .subscribe(deployment, &tier, 99_900, start_of_this_month(), None)
            .await;
        let path = installation.billing(&format!("statements/{}", this_period()));

        let (status, body) = get(&installation.app(installation.people.reader, None), &path).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let statement = json_of(&body);
        assert_eq!(statement["status"], "estimate");
        assert_eq!(statement["period"], this_period());
        assert_eq!(statement["hosted_invoice_url"], Value::Null);
        let lines = statement["lines"].as_array().expect("lines");
        assert_eq!(lines.len(), 1, "{body}");
        assert_eq!(lines[0]["deployment_id"], deployment.to_string());
        assert_eq!(lines[0]["tier"], tier);
        assert_eq!(lines[0]["monthly_price"], 99_900);
        let amount = lines[0]["amount"].as_i64().expect("an amount");
        assert!(amount > 0 && amount < 99_900, "{amount}");
        assert!(lines[0]["covered_seconds"].as_i64().expect("seconds") > 0);
        assert!(lines[0]["starts_at"].is_string() && lines[0]["ends_at"].is_string());
        assert_eq!(statement["total"], amount);

        assert_eq!(installation.statements_stored().await, 0);
        let (status, listed) = get(
            &installation.app(installation.people.reader, None),
            &installation.billing("statements"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let listed = json_of(&listed);
        assert_eq!(listed["data"], json!([]));
        assert_eq!(
            listed["current_period"],
            json!(BillingPeriod::containing(Utc::now()).to_string())
        );
    })
    .await;
}

#[tokio::test]
async fn the_estimate_bills_nothing_before_the_first_billing_period() {
    scenario(async |installation| {
        let tier = installation.paid_tier(99_900).await;
        installation
            .subscribe(Uuid::new_v4(), &tier, 99_900, start_of_this_month(), None)
            .await;
        let app = installation.app(
            installation.people.reader,
            Some(runtime(
                "http://127.0.0.1:1".to_string(),
                far_first_period(),
            )),
        );

        let (status, body) = get(
            &app,
            &installation.billing(&format!("statements/{}", this_period())),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let statement = json_of(&body);
        assert_eq!(statement["status"], "estimate");
        assert_eq!(statement["lines"], json!([]));
        assert_eq!(statement["total"], 0);
    })
    .await;
}

#[tokio::test]
async fn a_member_who_may_not_view_billing_reads_no_statement() {
    scenario(async |installation| {
        installation
            .statement(
                "2025-03",
                "paid",
                Some(("in_private", "https://x.test/i")),
                4900,
            )
            .await;

        for refused in [installation.people.watcher, installation.people.nobody] {
            let app = installation.app(refused, None);

            for path in [
                installation.billing("statements"),
                installation.billing(&format!("statements/{}", this_period())),
                installation.billing("statements/2025-03"),
            ] {
                let (status, body) = get(&app, &path).await;

                assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn no_response_carries_a_provider_key_or_identifier() {
    scenario(async |installation| {
        let customer = format!("cus_hidden_1_{}", installation.tag);
        let invoice = format!("in_hidden_1_{}", installation.tag);
        let server = MockServer::start();
        let created = server.mock(|when, then| {
            when.method(POST).path("/v1/customers");
            then.status(200).json_body(json!({"id": customer}));
        });
        let updated = server.mock(|when, then| {
            when.method(POST).path(format!("/v1/customers/{customer}"));
            then.status(200).json_body(json!({"id": customer}));
        });
        let session = server.mock(|when, then| {
            when.method(POST).path("/v1/checkout/sessions");
            then.status(200).json_body(json!({
                "id": "cs_test_1",
                "url": "https://checkout.stripe.test/c/pay/cs_test_1"
            }));
        });
        installation
            .statement(
                "2025-03",
                "sent",
                Some((invoice.as_str(), "https://invoice.stripe.test/i/hidden")),
                4900,
            )
            .await;
        let app = installation.app(
            installation.people.manager,
            Some(runtime(server.base_url(), far_first_period())),
        );

        let (put, saved) = call(
            &app,
            Method::PUT,
            &installation.billing("account"),
            Some(account_body("Acme SAS")),
        )
        .await;
        let (again, saved_again) = call(
            &app,
            Method::PUT,
            &installation.billing("account"),
            Some(account_body("Acme Holding SAS")),
        )
        .await;
        let (read, account) = get(&app, &installation.billing("account")).await;
        let (opened, opened_body) = call(
            &app,
            Method::POST,
            &installation.billing("payment-method-session"),
            None,
        )
        .await;
        let (listed, statements) = get(&app, &installation.billing("statements")).await;
        let (one, statement) = get(&app, &installation.billing("statements/2025-03")).await;

        assert_eq!(put, StatusCode::OK, "{saved}");
        assert_eq!(again, StatusCode::OK, "{saved_again}");
        assert_eq!(read, StatusCode::OK, "{account}");
        assert_eq!(opened, StatusCode::OK, "{opened_body}");
        assert_eq!(listed, StatusCode::OK, "{statements}");
        assert_eq!(one, StatusCode::OK, "{statement}");
        created.assert_hits(1);
        updated.assert_hits(1);
        session.assert_hits(1);
        assert_eq!(
            json_of(&opened_body),
            json!({"url": "https://checkout.stripe.test/c/pay/cs_test_1"})
        );
        assert_eq!(json_of(&account)["name"], "Acme Holding SAS");
        assert!(statements.contains("https://invoice.stripe.test/i/hidden"));
        assert!(statement.contains("https://invoice.stripe.test/i/hidden"));
        assert_eq!(
            installation.stored_account().await,
            Some((
                "Acme Holding SAS".to_string(),
                Some(customer.clone()),
                "missing".to_string()
            ))
        );
        for answer in [
            &saved,
            &saved_again,
            &account,
            &opened_body,
            &statements,
            &statement,
        ] {
            assert!(!answer.contains("cus_hidden_1"), "{answer}");
            assert!(!answer.contains("in_hidden_1"), "{answer}");
            assert!(!answer.contains(SECRET_KEY), "{answer}");
            assert!(!answer.contains(WEBHOOK_SECRET), "{answer}");
        }
    })
    .await;
}

#[tokio::test]
async fn statements_list_newest_first_and_show_the_invoice_only_while_it_is_unpaid() {
    scenario(async |installation| {
        installation
            .statement(
                "2025-02",
                "paid",
                Some(("in_paid", "https://invoice.test/paid")),
                1200,
            )
            .await;
        installation
            .statement(
                "2025-03",
                "sent",
                Some(("in_sent", "https://invoice.test/sent")),
                4900,
            )
            .await;
        installation
            .statement(
                "2025-01",
                "uncollectible",
                Some(("in_lost", "https://invoice.test/lost")),
                700,
            )
            .await;
        installation.statement("2024-12", "zero", None, 0).await;
        let app = installation.app(installation.people.reader, None);

        let (status, body) = get(&app, &installation.billing("statements")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let listed = json_of(&body);
        let data = listed["data"].as_array().expect("data");
        let summary: Vec<(&str, &str, i64, Option<&str>)> = data
            .iter()
            .map(|entry| {
                (
                    entry["period"].as_str().expect("period"),
                    entry["status"].as_str().expect("status"),
                    entry["total"].as_i64().expect("total"),
                    entry["hosted_invoice_url"].as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("2025-03", "sent", 4900, Some("https://invoice.test/sent")),
                ("2025-02", "paid", 1200, None),
                (
                    "2025-01",
                    "uncollectible",
                    700,
                    Some("https://invoice.test/lost")
                ),
                ("2024-12", "zero", 0, None),
            ]
        );
        assert!(!body.contains("https://invoice.test/paid"), "{body}");
        assert!(!body.contains("in_sent"), "{body}");

        let (status, body) = get(&app, &installation.billing("statements/2025-03")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let sent = json_of(&body);
        assert_eq!(sent["status"], "sent");
        assert_eq!(sent["period"], "2025-03");
        assert_eq!(sent["total"], 4900);
        assert_eq!(sent["hosted_invoice_url"], "https://invoice.test/sent");
        assert_eq!(sent["lines"].as_array().expect("lines").len(), 1);
        assert_eq!(sent["lines"][0]["amount"], 4900);
        assert_eq!(sent["lines"][0]["covered_seconds"], 28 * 86_400);

        let (status, body) = get(&app, &installation.billing("statements/2025-02")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(json_of(&body)["status"], "paid");
        assert_eq!(json_of(&body)["hosted_invoice_url"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn an_unknown_period_answers_404_and_a_malformed_one_422() {
    scenario(async |installation| {
        let app = installation.app(installation.people.reader, None);

        let (status, body) = get(&app, &installation.billing("statements/2025-07")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        for malformed in ["2025-13", "2025-1", "march", "2025-03-01", "202503"] {
            let (status, body) = get(
                &app,
                &installation.billing(&format!("statements/{malformed}")),
            )
            .await;

            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{malformed}: {body}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn with_no_selectable_tier_the_list_is_empty() {
    scenario(async |installation| {
        installation.paid_tier(4900).await;

        for allowed in [installation.people.watcher, installation.people.owner] {
            let (status, body) = get(
                &installation.app(allowed, None),
                &installation.billing("tiers"),
            )
            .await;

            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(json_of(&body), json!({"data": []}));
        }

        let (status, body) = get(
            &installation.app(installation.people.nobody, None),
            &installation.billing("tiers"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    })
    .await;
}

#[tokio::test]
async fn a_requested_tier_is_ignored_while_no_tier_is_selectable() {
    scenario(async |installation| {
        installation.serve_region().await;

        let (status, created) = installation.deploy("tiered", Some("anything-valid")).await;

        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().expect("an id");
        assert_eq!(installation.tier_of(id).await, "legacy-standard");
    })
    .await;
}

#[tokio::test]
async fn a_tier_code_that_is_not_one_is_refused_and_nothing_is_created() {
    scenario(async |installation| {
        installation.serve_region().await;

        let (status, refused) = installation.deploy("tiered", Some("Not Valid")).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(installation.deployments_stored().await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_legacy_deployment_opens_every_feature() {
    scenario(async |installation| {
        installation.serve_region().await;
        let (status, created) = installation.deploy("legacy", None).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().expect("an id").to_string();
        let app = installation.app(installation.people.watcher, None);

        let (status, body) = get(&app, &installation.deployment(&id, "features")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let features = json_of(&body);
        assert_eq!(features["tier"], "legacy-standard");
        let listed: Vec<(&str, bool)> = features["features"]
            .as_array()
            .expect("features")
            .iter()
            .map(|entry| {
                (
                    entry["feature"].as_str().expect("feature"),
                    entry["open"].as_bool().expect("open"),
                )
            })
            .collect();
        assert_eq!(
            listed,
            ALL_FEATURES
                .iter()
                .map(|name| (*name, true))
                .collect::<Vec<_>>()
        );

        let (status, body) = get(&app, &installation.deployment(&id, "subscription")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let subscription = json_of(&body);
        assert_eq!(subscription["tier"], "legacy-standard");
        assert_eq!(subscription["monthly_price"], 0);
        assert_eq!(subscription["offer"], "standard");
        assert_eq!(subscription["features"], json!(ALL_FEATURES));
        assert!(subscription["starts_at"].is_string());
        assert_eq!(subscription["ends_at"], Value::Null);

        let (status, body) = get(
            &installation.app(installation.people.nobody, None),
            &installation.deployment(&id, "features"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = get(
            &app,
            &installation.deployment(&Uuid::new_v4().to_string(), "features"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    })
    .await;
}

#[tokio::test]
async fn a_subscription_only_moves_to_a_tier_of_the_catalogue() {
    scenario(async |installation| {
        installation.serve_region().await;
        let (_, created) = installation.deploy("moving", None).await;
        let id = created["data"]["id"].as_str().expect("an id").to_string();
        let app = installation.app(installation.people.manager, None);
        let path = installation.deployment(&id, "subscription");

        let (unknown, unknown_body) = call(
            &app,
            Method::PUT,
            &path,
            Some(json!({"tier": "ghost-2026"})),
        )
        .await;
        let (legacy, legacy_body) = call(
            &app,
            Method::PUT,
            &path,
            Some(json!({"tier": "legacy-free"})),
        )
        .await;
        let (malformed, malformed_body) =
            call(&app, Method::PUT, &path, Some(json!({"tier": "Not Valid"}))).await;

        assert_eq!(unknown, StatusCode::NOT_FOUND, "{unknown_body}");
        assert_eq!(legacy, StatusCode::UNPROCESSABLE_ENTITY, "{legacy_body}");
        assert_eq!(
            malformed,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{malformed_body}"
        );
        assert_eq!(installation.tier_of(&id).await, "legacy-standard");
    })
    .await;
}

#[tokio::test]
async fn without_billing_the_session_answers_503_and_the_account_is_kept_without_the_provider() {
    scenario(async |installation| {
        let app = installation.app(installation.people.manager, None);

        let (status, body) = call(
            &app,
            Method::POST,
            &installation.billing("payment-method-session"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

        let (status, body) = call(
            &app,
            Method::PUT,
            &installation.billing("account"),
            Some(account_body("Acme SAS")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let account = json_of(&body);
        assert_eq!(account["name"], "Acme SAS");
        assert_eq!(account["address"]["country"], "FR");
        assert_eq!(account["vat_number"], "FR12345678901");
        assert_eq!(account["siren"], "123456789");
        assert_eq!(account["payment_method"], "missing");
        assert_eq!(account["complete"], false);
        assert_eq!(
            installation.stored_account().await,
            Some(("Acme SAS".to_string(), None, "missing".to_string()))
        );

        let (status, body) = call(
            &app,
            Method::PUT,
            &installation.billing("account"),
            Some(account_body("Acme Holding SAS")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            installation.stored_account().await,
            Some(("Acme Holding SAS".to_string(), None, "missing".to_string()))
        );

        let (status, body) = get(&app, &installation.billing("account")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(json_of(&body)["name"], "Acme Holding SAS");
    })
    .await;
}

#[tokio::test]
async fn details_that_are_not_valid_are_refused_and_nothing_is_stored() {
    scenario(async |installation| {
        let app = installation.app(installation.people.manager, None);

        for (field, value) in [
            ("email", json!("not-an-email")),
            ("name", json!("   ")),
            ("vat_number", json!("12")),
            ("siren", json!("12345")),
        ] {
            let mut body = account_body("Acme SAS");
            body[field] = value;

            let (status, answer) = call(
                &app,
                Method::PUT,
                &installation.billing("account"),
                Some(body),
            )
            .await;

            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{field}: {answer}"
            );
        }

        let mut body = account_body("Acme SAS");
        body["address"]["country"] = json!("France");
        let (status, answer) = call(
            &app,
            Method::PUT,
            &installation.billing("account"),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");

        assert_eq!(installation.stored_account().await, None);
    })
    .await;
}

#[tokio::test]
async fn a_session_needs_an_account_first() {
    scenario(async |installation| {
        let server = MockServer::start();
        let provider = server.mock(|when, then| {
            when.method(POST);
            then.status(200).json_body(json!({"id": "cus_x"}));
        });
        let app = installation.app(
            installation.people.manager,
            Some(runtime(server.base_url(), far_first_period())),
        );

        let (status, body) = call(
            &app,
            Method::POST,
            &installation.billing("payment-method-session"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        provider.assert_hits(0);
    })
    .await;
}
