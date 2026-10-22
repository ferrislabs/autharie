use chrono::{Duration, SecondsFormat, Utc};
use http::request::Parts;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{ApiClient, Fault};

pub const TOOL_NAMES: [&str; 10] = [
    "list_organisations",
    "list_deployments",
    "get_deployment",
    "get_recent_logs",
    "list_backups",
    "create_deployment",
    "upgrade_deployment",
    "list_webhooks",
    "create_webhook",
    "delete_webhook",
];

const COUNTER_METRICS: [&str; 3] = ["requests", "token_events", "logins"];
const DEFAULT_AVAILABILITY_DAYS: u32 = 7;
const DEFAULT_LOG_MINUTES: i64 = 15;
const MAX_LOG_MINUTES: i64 = 24 * 60;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct OrganisationArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeploymentArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the deployment; see list_deployments.")]
    pub deployment_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GetDeploymentArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the deployment; see list_deployments.")]
    pub deployment_id: String,
    #[schemars(description = "Days of availability history to include, 1 to 90. Default 7.")]
    pub availability_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
pub enum LevelFloor {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LevelFloor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RecentLogsArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the deployment; see list_deployments.")]
    pub deployment_id: String,
    #[schemars(description = "How far back to look, in minutes, 1 to 1440. Default 15.")]
    pub minutes: Option<i64>,
    #[schemars(
        description = "Lowest severity to include: trace, debug, info, warn, error or fatal. Default info."
    )]
    pub level_floor: Option<LevelFloor>,
    #[schemars(description = "Free text matched against the message of each line.")]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Ferriskey,
    Keycloak,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Production,
    Staging,
    Development,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
pub enum Offer {
    Free,
    Sandbox,
    Standard,
    Scale,
    Private,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CreateDeploymentArgs {
    #[schemars(description = "UUID of the organisation that will own the deployment.")]
    pub organisation_id: String,
    #[schemars(description = "Name of the deployment.")]
    pub name: String,
    #[schemars(description = "Identity product to deploy: ferriskey or keycloak.")]
    pub kind: Kind,
    #[schemars(description = "Product version to install, for example 0.9.0.")]
    pub version: String,
    #[schemars(description = "production, staging or development.")]
    pub environment: Environment,
    #[schemars(
        description = "What is being bought: free, sandbox, standard, scale or private. It decides the size and whether the deployment gets a cluster of its own."
    )]
    pub offer: Offer,
    #[schemars(description = "Pricing tier identifier, when the offer has several.")]
    pub tier: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UpgradeDeploymentArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the deployment; see list_deployments.")]
    pub deployment_id: String,
    #[schemars(description = "Target version as a semver, for example 0.10.0.")]
    pub version: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CreateWebhookArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the deployment; see list_deployments.")]
    pub deployment_id: String,
    #[schemars(description = "Realm of the deployment whose events are delivered.")]
    pub realm: String,
    #[schemars(description = "Name of the webhook.")]
    pub name: String,
    #[schemars(description = "What the webhook is for.")]
    pub description: Option<String>,
    #[schemars(description = "https URL that receives the deliveries.")]
    pub endpoint: String,
    #[schemars(description = "Event names to deliver, for example user.created.")]
    pub events: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeleteWebhookArgs {
    #[schemars(description = "UUID of the organisation; see list_organisations.")]
    pub organisation_id: String,
    #[schemars(description = "UUID of the deployment; see list_deployments.")]
    pub deployment_id: String,
    #[schemars(description = "UUID of the webhook; see list_webhooks.")]
    pub webhook_id: String,
}

#[derive(Clone)]
pub struct AutharieTools {
    api: ApiClient,
    tool_router: ToolRouter<Self>,
}

impl AutharieTools {
    pub fn new(api: ApiClient) -> Self {
        Self {
            api,
            tool_router: Self::tool_router(),
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }
}

pub fn bearer(parts: &Parts) -> Option<&str> {
    let header = parts
        .headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = header.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then_some(token.trim())
}

fn token(parts: &Parts) -> Result<&str, Fault> {
    bearer(parts).ok_or_else(|| Fault::new("access token missing"))
}

fn id(label: &str, value: &str) -> Result<Uuid, Fault> {
    Uuid::parse_str(value.trim()).map_err(|_| Fault::new(format!("{label} must be a UUID")))
}

fn finish(outcome: Result<Value, Fault>) -> CallToolResult {
    match outcome {
        Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
        Err(fault) => CallToolResult::error(vec![ContentBlock::text(fault.to_string())]),
    }
}

fn stamp(moment: chrono::DateTime<Utc>) -> String {
    moment.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn section(outcome: Result<Value, Fault>) -> Value {
    outcome.unwrap_or_else(|fault| json!({"error": fault.message}))
}

fn metric_summary(metric: &str, outcome: Result<Value, Fault>) -> Value {
    let series = match outcome {
        Ok(body) => body,
        Err(fault) => return json!({"error": fault.message}),
    };
    let values: Vec<u64> = series["data"]
        .as_array()
        .map(|points| points.iter().filter_map(|p| p["value"].as_u64()).collect())
        .unwrap_or_default();
    if COUNTER_METRICS.contains(&metric) {
        json!({"points": values.len(), "total": values.iter().sum::<u64>()})
    } else {
        json!({"points": values.len(), "latest": values.last()})
    }
}

#[tool_router(router = tool_router)]
impl AutharieTools {
    #[tool(
        description = "List the organisations the caller belongs to. Returns each organisation's id and name; the id is the organisation_id argument of every other tool.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_organisations(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        finish(
            async {
                self.api
                    .get(token(&parts)?, "users/@me/organisations", &[])
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "List the deployments of an organisation with their status, kind, version and environment.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_deployments(
        &self,
        Parameters(args): Parameters<OrganisationArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                self.api
                    .get(
                        token(&parts)?,
                        &format!("organisations/{organisation}/deployments"),
                        &[],
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Get one deployment: its record and status, its day-by-day availability, and a summary of the last 24 hours of usage (requests, token_events, logins as totals; active_users as the latest value). A section that cannot be read carries an error instead of data.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_deployment(
        &self,
        Parameters(args): Parameters<GetDeploymentArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let token = token(&parts)?;
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                let days = args
                    .availability_days
                    .unwrap_or(DEFAULT_AVAILABILITY_DAYS);
                if !(1..=90).contains(&days) {
                    return Err(Fault::new("availability_days must be between 1 and 90"));
                }
                let base = format!("organisations/{organisation}/deployments/{deployment}");
                let now = Utc::now();
                let window = [
                    ("from", stamp(now - Duration::hours(24))),
                    ("until", stamp(now)),
                ];
                let usage = |metric: &'static str| {
                    let path = format!("{base}/usage-metrics/{metric}");
                    let window = window.clone();
                    async move { (metric, self.api.get(token, &path, &window).await) }
                };
                let availability_path = format!("{base}/availability");
                let availability_query = [("days", days.to_string())];
                let (record, availability, requests, token_events, logins, active_users) = tokio::join!(
                    self.api.get(token, &base, &[]),
                    self.api.get(token, &availability_path, &availability_query),
                    usage("requests"),
                    usage("token_events"),
                    usage("logins"),
                    usage("active_users"),
                );
                let record = record?;
                let mut summary = serde_json::Map::new();
                for (metric, outcome) in [requests, token_events, logins, active_users] {
                    summary.insert(metric.to_owned(), metric_summary(metric, outcome));
                }
                Ok(json!({
                    "deployment": record,
                    "availability": section(availability),
                    "usage_last_24h": summary,
                }))
            }
            .await,
        )
    }

    #[tool(
        description = "Search the recent indexed logs of a deployment, newest first. Looks back a number of minutes (default 15, at most 1440) at or above a severity floor, optionally filtered by free text. Needs log indexing to be configured for the installation.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_recent_logs(
        &self,
        Parameters(args): Parameters<RecentLogsArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let token = token(&parts)?;
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                let minutes = args.minutes.unwrap_or(DEFAULT_LOG_MINUTES);
                if !(1..=MAX_LOG_MINUTES).contains(&minutes) {
                    return Err(Fault::new("minutes must be between 1 and 1440"));
                }
                let now = Utc::now();
                let mut query = vec![
                    ("from", stamp(now - Duration::minutes(minutes))),
                    ("to", stamp(now)),
                    (
                        "level_floor",
                        args.level_floor
                            .unwrap_or(LevelFloor::Info)
                            .as_str()
                            .to_owned(),
                    ),
                    ("deployment_id", deployment.to_string()),
                ];
                if let Some(text) = args.query.filter(|text| !text.trim().is_empty()) {
                    query.push(("q", text));
                }
                self.api
                    .get(
                        token,
                        &format!("organisations/{organisation}/logs/search"),
                        &query,
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "List the backups of a deployment.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_backups(
        &self,
        Parameters(args): Parameters<DeploymentArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                self.api
                    .get(
                        token(&parts)?,
                        &format!("organisations/{organisation}/deployments/{deployment}/backups"),
                        &[],
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Create a deployment in an organisation. Provisioning starts at once and the deployment may be billed according to its offer. Returns the new deployment.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_deployment(
        &self,
        Parameters(args): Parameters<CreateDeploymentArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                let mut body = json!({
                    "name": args.name,
                    "kind": args.kind,
                    "version": args.version,
                    "environment": args.environment,
                    "offer": args.offer,
                });
                if let Some(tier) = args.tier {
                    body["tier"] = Value::String(tier);
                }
                self.api
                    .post(
                        token(&parts)?,
                        &format!("organisations/{organisation}/deployments"),
                        &body,
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Upgrade a deployment to a target version. The change is handed to the data plane and the deployment is not usable as before until it settles. Refused when the deployment is not settled or the version cannot be installed.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn upgrade_deployment(
        &self,
        Parameters(args): Parameters<UpgradeDeploymentArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                self.api
                    .post(
                        token(&parts)?,
                        &format!("organisations/{organisation}/deployments/{deployment}/upgrade"),
                        &json!({"version": args.version}),
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "List the webhooks of a deployment with their realm, endpoint, events and state (pending, active or failed). Secrets are never returned.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_webhooks(
        &self,
        Parameters(args): Parameters<DeploymentArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                self.api
                    .get(
                        token(&parts)?,
                        &format!("organisations/{organisation}/deployments/{deployment}/webhooks"),
                        &[],
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Create a webhook on a deployment's realm, delivering the named events to an https endpoint. The webhook starts pending until the deployment applies it. The signing secret is not returned.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_webhook(
        &self,
        Parameters(args): Parameters<CreateWebhookArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                let mut body = json!({
                    "realm": args.realm,
                    "name": args.name,
                    "endpoint": args.endpoint,
                    "events": args.events,
                });
                if let Some(description) = args.description {
                    body["description"] = Value::String(description);
                }
                self.api
                    .post(
                        token(&parts)?,
                        &format!("organisations/{organisation}/deployments/{deployment}/webhooks"),
                        &body,
                    )
                    .await
            }
            .await,
        )
    }

    #[tool(
        description = "Delete a webhook from a deployment. Deliveries to its endpoint stop and its signing secret is discarded.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_webhook(
        &self,
        Parameters(args): Parameters<DeleteWebhookArgs>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        finish(
            async {
                let organisation = id("organisation_id", &args.organisation_id)?;
                let deployment = id("deployment_id", &args.deployment_id)?;
                let webhook = id("webhook_id", &args.webhook_id)?;
                self.api
                    .delete(
                        token(&parts)?,
                        &format!(
                            "organisations/{organisation}/deployments/{deployment}/webhooks/{webhook}"
                        ),
                    )
                    .await
                    .map(|body| {
                        if body.is_null() {
                            json!({"deleted": true})
                        } else {
                            body
                        }
                    })
            }
            .await,
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AutharieTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Autharie tools. Call list_organisations first, then list_deployments, to find the ids the other tools take.",
        )
    }
}
