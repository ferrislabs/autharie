use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use autharie_mcp::{ApiClient, TOOL_NAMES, router};
use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rmcp::{
    ClientHandler, ClientLifecycleMode, ClientServiceExt,
    model::{CallToolRequestParams, CallToolResult, ProtocolVersion},
    service::{RoleClient, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};

const ORG: &str = "11111111-1111-4111-8111-111111111111";
const DEPLOYMENT: &str = "22222222-2222-4222-8222-222222222222";
const WEBHOOK: &str = "33333333-3333-4333-8333-333333333333";
const TOKEN: &str = "exchanged-token";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    query: HashMap<String, String>,
    authorization: String,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    log: Arc<Mutex<Vec<Seen>>>,
    replies: Arc<Mutex<HashMap<String, (u16, Value)>>>,
}

impl Fake {
    fn reply(&self, method: &str, path: &str, status: u16, body: Value) {
        self.replies
            .lock()
            .unwrap()
            .insert(format!("{method} {path}"), (status, body));
    }

    fn seen(&self) -> Vec<Seen> {
        self.log.lock().unwrap().clone()
    }

    fn only(&self) -> Seen {
        let seen = self.seen();
        assert_eq!(seen.len(), 1, "expected one API call, saw {seen:?}");
        seen[0].clone()
    }
}

async fn record(State(fake): State<Fake>, request: Request) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(url_pairs).unwrap_or_default();
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes: Bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    fake.log.lock().unwrap().push(Seen {
        method: method.clone(),
        path: path.clone(),
        query,
        authorization,
        body,
    });
    let reply = fake
        .replies
        .lock()
        .unwrap()
        .get(&format!("{method} {path}"))
        .cloned();
    match reply {
        Some((status, Value::Null)) => StatusCode::from_u16(status).unwrap().into_response(),
        Some((status, body)) => {
            (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({"code": "E_NOT_FOUND", "status": 404, "message": "no such route"})),
        )
            .into_response(),
    }
}

fn url_pairs(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_owned(), decode(v)))
        .collect()
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                out.push(u8::from_str_radix(&value[i + 1..i + 3], 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

#[derive(Clone, Default)]
struct Probe;

impl ClientHandler for Probe {}

struct Harness {
    fake: Fake,
    mcp: String,
    client: RunningService<RoleClient, Probe>,
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

async fn harness() -> Harness {
    let fake = Fake::default();
    let api = serve(Router::new().fallback(record).with_state(fake.clone())).await;
    let mcp = serve(router(ApiClient::new(api))).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("{mcp}/mcp")).auth_header(TOKEN),
    );
    let client = Probe
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .unwrap();
    Harness { fake, mcp, client }
}

impl Harness {
    async fn call(&self, tool: &'static str, arguments: Value) -> CallToolResult {
        let params = CallToolRequestParams::new(tool)
            .with_arguments(arguments.as_object().cloned().unwrap_or_default());
        self.client.call_tool(params).await.unwrap()
    }
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .map(|block| {
            serde_json::to_value(block).unwrap()["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn data(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "tool error: {}", text(result));
    serde_json::from_str(&text(result)).unwrap()
}

fn deployments(org: &str) -> String {
    format!("/organisations/{org}/deployments")
}

#[tokio::test]
async fn tools_list_names_the_curated_set_with_honest_annotations() {
    let h = harness().await;
    let tools = h.client.list_all_tools().await.unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    let mut expected = TOOL_NAMES.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);

    let hints = |name: &str| {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let a = tool.annotations.clone().unwrap();
        (a.read_only_hint, a.destructive_hint)
    };
    for read in [
        "list_organisations",
        "list_deployments",
        "get_deployment",
        "get_recent_logs",
        "list_backups",
        "list_webhooks",
    ] {
        assert_eq!(hints(read).0, Some(true), "{read}");
    }
    for write in ["create_deployment", "create_webhook"] {
        assert_eq!(hints(write), (Some(false), Some(false)), "{write}");
    }
    for destructive in ["upgrade_deployment", "delete_webhook"] {
        assert_eq!(
            hints(destructive),
            (Some(false), Some(true)),
            "{destructive}"
        );
    }
    for tool in &tools {
        assert!(
            tool.description.as_deref().is_some_and(|d| !d.is_empty()),
            "{}",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&json!("object")),
            "{}",
            tool.name
        );
    }
}

#[tokio::test]
async fn list_organisations_forwards_the_bearer_and_returns_the_body() {
    let h = harness().await;
    h.fake.reply(
        "GET",
        "/users/@me/organisations",
        200,
        json!({"data": [{"id": ORG}]}),
    );
    let result = h.call("list_organisations", json!({})).await;
    assert_eq!(data(&result), json!({"data": [{"id": ORG}]}));
    let seen = h.fake.only();
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.authorization, format!("Bearer {TOKEN}"));
}

#[tokio::test]
async fn list_deployments_maps_to_the_organisation_route() {
    let h = harness().await;
    h.fake
        .reply("GET", &deployments(ORG), 200, json!({"data": []}));
    let result = h
        .call("list_deployments", json!({"organisation_id": ORG}))
        .await;
    assert_eq!(data(&result), json!({"data": []}));
    let seen = h.fake.only();
    assert_eq!(seen.path, deployments(ORG));
    assert_eq!(seen.authorization, format!("Bearer {TOKEN}"));
}

#[tokio::test]
async fn get_deployment_gathers_status_availability_and_a_usage_summary() {
    let h = harness().await;
    let base = format!("{}/{DEPLOYMENT}", deployments(ORG));
    h.fake.reply(
        "GET",
        &base,
        200,
        json!({"data": {"id": DEPLOYMENT, "status": "running"}}),
    );
    h.fake.reply(
        "GET",
        &format!("{base}/availability"),
        200,
        json!({"data": {"days": []}}),
    );
    h.fake.reply(
        "GET",
        &format!("{base}/usage-metrics/requests"),
        200,
        json!({"data": [{"bucket": "t1", "value": 3}, {"bucket": "t2", "value": 4}]}),
    );
    h.fake.reply(
        "GET",
        &format!("{base}/usage-metrics/logins"),
        200,
        json!({"data": []}),
    );
    h.fake.reply(
        "GET",
        &format!("{base}/usage-metrics/active_users"),
        200,
        json!({"data": [{"bucket": "t1", "value": 9}, {"bucket": "t2", "value": 12}]}),
    );
    h.fake.reply(
        "GET",
        &format!("{base}/usage-metrics/token_events"),
        403,
        json!({"code": "E_FORBIDDEN", "status": 403, "message": "nope"}),
    );
    let result = h
        .call(
            "get_deployment",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT, "availability_days": 30}),
        )
        .await;
    let body = data(&result);
    assert_eq!(body["deployment"]["data"]["status"], "running");
    assert_eq!(body["availability"], json!({"data": {"days": []}}));
    let usage = &body["usage_last_24h"];
    assert_eq!(usage["requests"], json!({"points": 2, "total": 7}));
    assert_eq!(usage["logins"], json!({"points": 0, "total": 0}));
    assert_eq!(usage["active_users"], json!({"points": 2, "latest": 12}));
    assert_eq!(
        usage["token_events"],
        json!({"error": "Autharie API 403: nope"})
    );

    let seen = h.fake.seen();
    assert_eq!(seen.len(), 6);
    assert!(
        seen.iter()
            .all(|s| s.authorization == format!("Bearer {TOKEN}"))
    );
    let availability = seen
        .iter()
        .find(|s| s.path.ends_with("/availability"))
        .unwrap();
    assert_eq!(availability.query["days"], "30");
    let requests = seen.iter().find(|s| s.path.ends_with("/requests")).unwrap();
    assert!(requests.query.contains_key("from") && requests.query.contains_key("until"));
}

#[tokio::test]
async fn get_deployment_fails_as_a_tool_error_when_the_deployment_is_unreadable() {
    let h = harness().await;
    let base = format!("{}/{DEPLOYMENT}", deployments(ORG));
    h.fake.reply(
        "GET",
        &base,
        404,
        json!({"code": "E_NOT_FOUND", "status": 404, "message": "no such deployment"}),
    );
    let result = h
        .call(
            "get_deployment",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT}),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "Autharie API 404: no such deployment");
}

#[tokio::test]
async fn get_recent_logs_searches_the_window_with_the_floor_and_text() {
    let h = harness().await;
    h.fake.reply(
        "GET",
        &format!("/organisations/{ORG}/logs/search"),
        200,
        json!({"data": [{"message": "boom"}]}),
    );
    let result = h
        .call(
            "get_recent_logs",
            json!({
                "organisation_id": ORG, "deployment_id": DEPLOYMENT,
                "minutes": 30, "level_floor": "error", "query": "boom now"
            }),
        )
        .await;
    assert_eq!(data(&result), json!({"data": [{"message": "boom"}]}));
    let seen = h.fake.only();
    assert_eq!(seen.query["level_floor"], "error");
    assert_eq!(seen.query["deployment_id"], DEPLOYMENT);
    assert_eq!(seen.query["q"], "boom now");
    assert!(seen.query["from"] < seen.query["to"]);
    assert_eq!(seen.authorization, format!("Bearer {TOKEN}"));
}

#[tokio::test]
async fn get_recent_logs_defaults_to_info_and_refuses_an_absurd_window() {
    let h = harness().await;
    h.fake.reply(
        "GET",
        &format!("/organisations/{ORG}/logs/search"),
        200,
        json!({"data": []}),
    );
    h.call(
        "get_recent_logs",
        json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT}),
    )
    .await;
    let seen = h.fake.only();
    assert_eq!(seen.query["level_floor"], "info");
    assert!(!seen.query.contains_key("q"));

    let refused = h
        .call(
            "get_recent_logs",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT, "minutes": 100000}),
        )
        .await;
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(h.fake.seen().len(), 1);
}

#[tokio::test]
async fn list_backups_maps_to_the_backups_route() {
    let h = harness().await;
    let path = format!("{}/{DEPLOYMENT}/backups", deployments(ORG));
    h.fake
        .reply("GET", &path, 200, json!({"data": [{"id": "b1"}]}));
    let result = h
        .call(
            "list_backups",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT}),
        )
        .await;
    assert_eq!(data(&result), json!({"data": [{"id": "b1"}]}));
    let seen = h.fake.only();
    assert_eq!(seen.path, path);
    assert_eq!(seen.authorization, format!("Bearer {TOKEN}"));
}

#[tokio::test]
async fn create_deployment_posts_the_request_body() {
    let h = harness().await;
    h.fake.reply(
        "POST",
        &deployments(ORG),
        201,
        json!({"data": {"id": DEPLOYMENT}}),
    );
    let result = h
        .call(
            "create_deployment",
            json!({
                "organisation_id": ORG, "name": "prod", "kind": "ferriskey", "version": "0.9.0",
                "environment": "production", "offer": "standard", "tier": "standard-2026"
            }),
        )
        .await;
    assert_eq!(data(&result), json!({"data": {"id": DEPLOYMENT}}));
    let seen = h.fake.only();
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.authorization, format!("Bearer {TOKEN}"));
    assert_eq!(
        seen.body,
        json!({
            "name": "prod", "kind": "ferriskey", "version": "0.9.0",
            "environment": "production", "offer": "standard", "tier": "standard-2026"
        })
    );
}

#[tokio::test]
async fn an_api_refusal_comes_back_as_a_tool_error_not_a_protocol_error() {
    let h = harness().await;
    h.fake.reply(
        "POST",
        &deployments(ORG),
        422,
        json!({"code": "E_UNPROCESSABLE", "status": 422, "message": "the offer is refused"}),
    );
    let result = h
        .call(
            "create_deployment",
            json!({
                "organisation_id": ORG, "name": "x", "kind": "ferriskey", "version": "0.9.0",
                "environment": "staging", "offer": "free"
            }),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "Autharie API 422: the offer is refused");
    assert!(!h.fake.only().body.as_object().unwrap().contains_key("tier"));
}

#[tokio::test]
async fn upgrade_deployment_posts_the_target_version() {
    let h = harness().await;
    let path = format!("{}/{DEPLOYMENT}/upgrade", deployments(ORG));
    h.fake
        .reply("POST", &path, 200, json!({"change": "upgrading"}));
    let result = h
        .call(
            "upgrade_deployment",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT, "version": "0.10.0"}),
        )
        .await;
    assert_eq!(data(&result), json!({"change": "upgrading"}));
    let seen = h.fake.only();
    assert_eq!(seen.path, path);
    assert_eq!(seen.body, json!({"version": "0.10.0"}));
    assert_eq!(seen.authorization, format!("Bearer {TOKEN}"));
}

#[tokio::test]
async fn webhooks_are_listed_created_and_deleted_through_the_deployment_routes() {
    let h = harness().await;
    let base = format!("{}/{DEPLOYMENT}/webhooks", deployments(ORG));
    h.fake.reply("GET", &base, 200, json!({"data": []}));
    h.fake
        .reply("POST", &base, 201, json!({"data": {"id": WEBHOOK}}));
    h.fake
        .reply("DELETE", &format!("{base}/{WEBHOOK}"), 204, Value::Null);

    let listed = h
        .call(
            "list_webhooks",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT}),
        )
        .await;
    assert_eq!(data(&listed), json!({"data": []}));

    let created = h
        .call(
            "create_webhook",
            json!({
                "organisation_id": ORG, "deployment_id": DEPLOYMENT, "realm": "master",
                "name": "users", "endpoint": "https://example.test/hook", "events": ["user.created"]
            }),
        )
        .await;
    assert_eq!(data(&created), json!({"data": {"id": WEBHOOK}}));

    let deleted = h
        .call(
            "delete_webhook",
            json!({"organisation_id": ORG, "deployment_id": DEPLOYMENT, "webhook_id": WEBHOOK}),
        )
        .await;
    assert_eq!(data(&deleted), json!({"deleted": true}));

    let seen = h.fake.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|s| s.authorization == format!("Bearer {TOKEN}"))
    );
    assert_eq!(
        seen[1].body,
        json!({
            "realm": "master", "name": "users",
            "endpoint": "https://example.test/hook", "events": ["user.created"]
        })
    );
    assert_eq!(
        (seen[2].method.as_str(), seen[2].path.as_str()),
        ("DELETE", format!("{base}/{WEBHOOK}").as_str())
    );
}

#[tokio::test]
async fn an_id_that_is_not_a_uuid_never_reaches_the_api() {
    let h = harness().await;
    let result = h
        .call(
            "list_deployments",
            json!({"organisation_id": "../platform/operators"}),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "organisation_id must be a UUID");
    assert!(h.fake.seen().is_empty());
}

#[tokio::test]
async fn a_request_without_a_bearer_is_refused_and_health_is_open() {
    let h = harness().await;
    let http = reqwest::Client::new();
    let refused = http
        .post(format!("{}/mcp", h.mcp))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
    assert_eq!(refused.headers()["www-authenticate"], "Bearer");
    let health = http.get(format!("{}/healthz", h.mcp)).send().await.unwrap();
    assert_eq!(health.status(), 200);
}
