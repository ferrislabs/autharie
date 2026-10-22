use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ferriskey_mcp::{TOOL_NAMES, Upstream, router};
use rmcp::{
    ClientHandler, ClientLifecycleMode, ClientServiceExt,
    model::{CallToolRequestParams, CallToolResult, ProtocolVersion},
    service::{RoleClient, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};

const NS: &str = "tenant-1";
const INSTANCE: &str = "acme";
const REALM: &str = "acme";
const USER: &str = "11111111-1111-4111-8111-111111111111";
const ROLE: &str = "22222222-2222-4222-8222-222222222222";
const CLIENT: &str = "33333333-3333-4333-8333-333333333333";
const ORG: &str = "44444444-4444-4444-8444-444444444444";
const GROUP: &str = "55555555-5555-4555-8555-555555555555";
const SESSION: &str = "66666666-6666-4666-8666-666666666666";

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
    token: String,
    client: RunningService<RoleClient, Probe>,
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

fn jwt(issuer: &str) -> String {
    let part = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
    format!(
        "{}.{}.signature",
        part(json!({"alg": "RS256"})),
        part(json!({"iss": issuer, "sub": "someone"}))
    )
}

fn mcp_url(mcp: &str, namespace: &str, name: &str) -> String {
    format!("{mcp}/instances/{namespace}/{name}/mcp")
}

async fn harness() -> Harness {
    harness_for(NS, INSTANCE, jwt("https://id.example.com/api/realms/acme")).await
}

async fn harness_for(namespace: &str, name: &str, token: String) -> Harness {
    let fake = Fake::default();
    let api = serve(Router::new().fallback(record).with_state(fake.clone())).await;
    let upstream = Upstream::new(&format!("{api}/i/{{namespace}}/{{name}}"), "/api").unwrap();
    let mcp = serve(router(upstream)).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(mcp_url(&mcp, namespace, name))
            .auth_header(token.clone()),
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
    Harness {
        fake,
        mcp,
        token,
        client,
    }
}

impl Harness {
    async fn call(&self, tool: &'static str, arguments: Value) -> CallToolResult {
        let params = CallToolRequestParams::new(tool)
            .with_arguments(arguments.as_object().cloned().unwrap_or_default());
        self.client.call_tool(params).await.unwrap()
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
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

fn route(rest: &str) -> String {
    format!("/i/{NS}/{INSTANCE}/api/realms/{REALM}/{rest}")
}

async fn read(tool: &'static str, arguments: Value, rest: &str) -> (Harness, Seen) {
    let h = harness().await;
    h.fake
        .reply("GET", &route(rest), 200, json!({"data": [{"id": "x"}]}));
    let result = h.call(tool, arguments).await;
    assert_eq!(data(&result), json!({"data": [{"id": "x"}]}));
    let seen = h.fake.only();
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.path, route(rest));
    assert_eq!(seen.authorization, h.bearer());
    (h, seen)
}

async fn write(
    tool: &'static str,
    arguments: Value,
    method: &str,
    rest: &str,
    status: u16,
    reply: Value,
) -> (Harness, Seen) {
    let h = harness().await;
    h.fake.reply(method, &route(rest), status, reply);
    let result = h.call(tool, arguments).await;
    assert_ne!(result.is_error, Some(true), "tool error: {}", text(&result));
    let seen = h.fake.only();
    assert_eq!(seen.method, method);
    assert_eq!(seen.path, route(rest));
    assert_eq!(seen.authorization, h.bearer());
    (h, seen)
}

fn existing_user() -> Value {
    json!({"data": {
        "id": USER,
        "username": "alice",
        "firstname": "Alice",
        "lastname": "Liddell",
        "email": "alice@example.com",
        "email_verified": true,
        "enabled": true,
        "required_actions": ["verify_email"]
    }})
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
    for name in [
        "list_realms",
        "list_users",
        "get_user",
        "get_user_roles",
        "list_roles",
        "list_clients",
        "get_client",
        "list_organisations",
        "list_groups",
        "list_group_members",
        "list_webhooks",
        "list_security_events",
        "list_user_sessions",
    ] {
        assert_eq!(hints(name).0, Some(true), "{name}");
    }
    for name in [
        "create_user",
        "update_user",
        "enable_user",
        "unlock_user",
        "assign_role",
        "add_group_member",
    ] {
        assert_eq!(hints(name), (Some(false), Some(false)), "{name}");
    }
    for name in [
        "disable_user",
        "require_user_actions",
        "unassign_role",
        "remove_group_member",
        "revoke_user_session",
    ] {
        assert_eq!(hints(name), (Some(false), Some(true)), "{name}");
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
        assert!(
            tool.input_schema["properties"].get("realm").is_some(),
            "{} takes a realm",
            tool.name
        );
    }
}

#[tokio::test]
async fn list_realms_asks_for_the_realms_the_caller_may_access() {
    let (_, seen) = read(
        "list_realms",
        json!({"search": "ac", "limit": 5}),
        "users/@me/realms",
    )
    .await;
    assert_eq!(seen.query["search"], "ac");
    assert_eq!(seen.query["limit"], "5");
}

#[tokio::test]
async fn list_users_passes_the_filters_and_the_page() {
    let (_, seen) = read(
        "list_users",
        json!({"search": "ali ce", "enabled": true, "role_id": ROLE, "page": 2, "limit": 10, "email": "a@b.c", "username": "al"}),
        "users",
    )
    .await;
    assert_eq!(seen.query["search"], "ali ce");
    assert_eq!(seen.query["enabled"], "true");
    assert_eq!(seen.query["role_id"], ROLE);
    assert_eq!(seen.query["page"], "2");
    assert_eq!(seen.query["limit"], "10");
    assert_eq!(seen.query["email"], "a@b.c");
    assert_eq!(seen.query["username"], "al");
}

#[tokio::test]
async fn get_user_and_its_roles_map_to_the_user_routes() {
    read(
        "get_user",
        json!({"user_id": USER}),
        &format!("users/{USER}"),
    )
    .await;
    read(
        "get_user_roles",
        json!({"user_id": USER}),
        &format!("users/{USER}/roles"),
    )
    .await;
}

#[tokio::test]
async fn list_roles_and_organisations_and_webhooks_map_to_their_routes() {
    let (_, seen) = read("list_roles", json!({"search": "admin"}), "roles").await;
    assert_eq!(seen.query["search"], "admin");
    read("list_organisations", json!({}), "organizations").await;
    let (_, seen) = read("list_webhooks", json!({"limit": 3}), "webhooks").await;
    assert_eq!(seen.query["limit"], "3");
}

#[tokio::test]
async fn clients_are_listed_and_read_without_their_secrets() {
    let h = harness().await;
    h.fake.reply(
        "GET",
        &route("clients"),
        200,
        json!({"data": [{"id": CLIENT, "client_id": "app", "secret": "s3cret", "name": "App"}]}),
    );
    h.fake.reply(
        "GET",
        &route(&format!("clients/{CLIENT}")),
        200,
        json!({"data": {"id": CLIENT, "client_id": "app", "client_secret": "s3cret", "nested": {"secret": "x", "kept": 1}}}),
    );
    let listed = h
        .call("list_clients", json!({"search": "app", "enabled": false}))
        .await;
    let body = data(&listed);
    assert_eq!(body["data"][0]["client_id"], "app");
    assert!(body["data"][0].get("secret").is_none());
    assert!(!text(&listed).contains("s3cret"));
    let got = h.call("get_client", json!({"client_id": CLIENT})).await;
    let body = data(&got);
    assert_eq!(body["data"]["nested"], json!({"kept": 1}));
    assert!(!text(&got).contains("s3cret"));
    let seen = h.fake.seen();
    assert_eq!(seen[0].query["search"], "app");
    assert_eq!(seen[0].query["enabled"], "false");
    assert_eq!(seen[1].path, route(&format!("clients/{CLIENT}")));
    assert!(seen.iter().all(|s| s.authorization == h.bearer()));
}

#[tokio::test]
async fn groups_and_members_are_read_through_the_organisation() {
    let (_, seen) = read(
        "list_groups",
        json!({"organisation_id": ORG, "search": "support"}),
        &format!("organizations/{ORG}/groups"),
    )
    .await;
    assert_eq!(seen.query["search"], "support");
    read(
        "list_group_members",
        json!({"organisation_id": ORG, "group_id": GROUP, "page": 3}),
        &format!("organizations/{ORG}/groups/{GROUP}/members"),
    )
    .await;
}

#[tokio::test]
async fn security_events_come_newest_first_with_filters() {
    let (_, seen) = read(
        "list_security_events",
        json!({
            "event_types": ["login_failure", "session_revoked"],
            "status": "failure",
            "actor_id": USER,
            "from_timestamp": "2026-01-01T00:00:00Z",
            "to_timestamp": "2026-01-02T00:00:00Z",
            "ip_address": "10.0.",
            "limit": 50
        }),
        "seawatch/v1/security-events",
    )
    .await;
    assert_eq!(seen.query["order"], "desc");
    assert_eq!(seen.query["event_types"], "login_failure,session_revoked");
    assert_eq!(seen.query["status"], "failure");
    assert_eq!(seen.query["actor_id"], USER);
    assert_eq!(seen.query["from_timestamp"], "2026-01-01T00:00:00Z");
    assert_eq!(seen.query["to_timestamp"], "2026-01-02T00:00:00Z");
    assert_eq!(seen.query["ip_address"], "10.0.");
    assert_eq!(seen.query["limit"], "50");
}

#[tokio::test]
async fn user_sessions_are_listed() {
    read(
        "list_user_sessions",
        json!({"user_id": USER}),
        &format!("users/{USER}/sessions"),
    )
    .await;
}

#[tokio::test]
async fn create_user_posts_only_the_given_fields() {
    let (_, seen) = write(
        "create_user",
        json!({"username": "bob", "email": "bob@example.com", "email_verified": false}),
        "POST",
        "users",
        201,
        json!({"data": {"id": USER, "username": "bob"}}),
    )
    .await;
    assert_eq!(
        seen.body,
        json!({"username": "bob", "email": "bob@example.com", "email_verified": false})
    );
}

#[tokio::test]
async fn update_user_keeps_what_the_caller_did_not_name() {
    let h = harness().await;
    let path = route(&format!("users/{USER}"));
    h.fake.reply("GET", &path, 200, existing_user());
    h.fake.reply("PUT", &path, 200, existing_user());
    let result = h
        .call(
            "update_user",
            json!({"user_id": USER, "lastname": "Hargreaves"}),
        )
        .await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let seen = h.fake.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].authorization, h.bearer());
    assert_eq!(
        seen[1].body,
        json!({
            "firstname": "Alice",
            "lastname": "Hargreaves",
            "email": "alice@example.com",
            "email_verified": true,
            "enabled": true
        })
    );
}

#[tokio::test]
async fn disable_and_enable_flip_only_the_enabled_flag() {
    for (tool, enabled) in [("disable_user", false), ("enable_user", true)] {
        let h = harness().await;
        let path = route(&format!("users/{USER}"));
        h.fake.reply("GET", &path, 200, existing_user());
        h.fake.reply("PUT", &path, 200, existing_user());
        let result = h.call(tool, json!({"user_id": USER})).await;
        assert_ne!(result.is_error, Some(true), "{}", text(&result));
        let seen = h.fake.seen();
        assert_eq!(seen[1].method, "PUT");
        assert_eq!(seen[1].body["enabled"], json!(enabled), "{tool}");
        assert_eq!(seen[1].body["firstname"], "Alice");
        assert_eq!(seen[1].body["email"], "alice@example.com");
        assert!(seen[1].body.get("required_actions").is_none());
    }
}

#[tokio::test]
async fn require_user_actions_replaces_the_list_and_refuses_unknown_actions() {
    let h = harness().await;
    let path = route(&format!("users/{USER}"));
    h.fake.reply("GET", &path, 200, existing_user());
    h.fake.reply("PUT", &path, 200, existing_user());
    let result = h
        .call(
            "require_user_actions",
            json!({"user_id": USER, "actions": ["update_password", "configure_otp"]}),
        )
        .await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let seen = h.fake.seen();
    assert_eq!(
        seen[1].body["required_actions"],
        json!(["update_password", "configure_otp"])
    );
    assert_eq!(seen[1].body["enabled"], true);

    let before = h.fake.seen().len();
    let params = CallToolRequestParams::new("require_user_actions").with_arguments(
        json!({"user_id": USER, "actions": ["delete_everything"]})
            .as_object()
            .cloned()
            .unwrap(),
    );
    let refused = h.client.call_tool(params).await;
    assert!(refused.is_err() || refused.unwrap().is_error == Some(true));
    assert_eq!(h.fake.seen().len(), before);
}

#[tokio::test]
async fn unlock_user_posts_to_the_unlock_route() {
    let (_, seen) = write(
        "unlock_user",
        json!({"user_id": USER}),
        "POST",
        &format!("users/{USER}/unlock"),
        204,
        Value::Null,
    )
    .await;
    assert_eq!(seen.body, Value::Null);
}

#[tokio::test]
async fn roles_are_assigned_and_unassigned_on_the_user() {
    write(
        "assign_role",
        json!({"user_id": USER, "role_id": ROLE}),
        "POST",
        &format!("users/{USER}/roles/{ROLE}"),
        200,
        json!({"message": "ok"}),
    )
    .await;
    write(
        "unassign_role",
        json!({"user_id": USER, "role_id": ROLE}),
        "DELETE",
        &format!("users/{USER}/roles/{ROLE}"),
        200,
        json!({"message": "ok"}),
    )
    .await;
}

#[tokio::test]
async fn group_members_are_added_and_removed() {
    let (_, seen) = write(
        "add_group_member",
        json!({"organisation_id": ORG, "group_id": GROUP, "user_id": USER}),
        "POST",
        &format!("organizations/{ORG}/groups/{GROUP}/members"),
        201,
        json!({"user_id": USER}),
    )
    .await;
    assert_eq!(seen.body, json!({"user_id": USER}));
    write(
        "remove_group_member",
        json!({"organisation_id": ORG, "group_id": GROUP, "user_id": USER}),
        "DELETE",
        &format!("organizations/{ORG}/groups/{GROUP}/members/{USER}"),
        204,
        Value::Null,
    )
    .await;
}

#[tokio::test]
async fn a_session_is_revoked_and_an_empty_answer_reads_as_done() {
    let h = harness().await;
    h.fake.reply(
        "DELETE",
        &route(&format!("users/{USER}/sessions/{SESSION}")),
        204,
        Value::Null,
    );
    let result = h
        .call(
            "revoke_user_session",
            json!({"user_id": USER, "session_id": SESSION}),
        )
        .await;
    assert_eq!(data(&result), json!({"ok": true}));
    assert_eq!(h.fake.only().method, "DELETE");
}

#[tokio::test]
async fn the_realm_defaults_to_the_tokens_and_an_argument_overrides_it() {
    let h = harness().await;
    h.fake.reply(
        "GET",
        &format!("/i/{NS}/{INSTANCE}/api/realms/other/roles"),
        200,
        json!({"data": []}),
    );
    let result = h.call("list_roles", json!({"realm": "other"})).await;
    assert_eq!(data(&result), json!({"data": []}));
    assert_eq!(
        h.fake.only().path,
        format!("/i/{NS}/{INSTANCE}/api/realms/other/roles")
    );
}

#[tokio::test]
async fn a_token_without_a_realm_needs_the_argument() {
    let h = harness_for(NS, INSTANCE, "opaque-token".to_owned()).await;
    let result = h.call("list_roles", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        text(&result),
        "realm is required: the token does not name one"
    );
    assert!(h.fake.seen().is_empty());
}

#[tokio::test]
async fn a_realm_that_could_change_the_path_never_reaches_the_api() {
    let h = harness().await;
    for realm in ["../master", "a/b", "..", ".", "a b", "", "x?y=1", "%2e%2e"] {
        let result = h.call("list_roles", json!({"realm": realm})).await;
        assert_eq!(result.is_error, Some(true), "{realm}");
    }
    assert!(h.fake.seen().is_empty());
}

#[tokio::test]
async fn a_forged_issuer_cannot_steer_the_call_off_its_realm_route() {
    let h = harness_for(
        NS,
        INSTANCE,
        jwt("https://id.example.com/api/realms/../../admin"),
    )
    .await;
    let result = h.call("list_roles", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(h.fake.seen().is_empty());
}

#[tokio::test]
async fn an_id_that_is_not_a_uuid_never_reaches_the_api() {
    let h = harness().await;
    let result = h
        .call("get_user", json!({"user_id": "../../realms/master/users"}))
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "user_id must be a UUID");
    assert!(h.fake.seen().is_empty());
}

#[tokio::test]
async fn an_api_refusal_comes_back_as_a_tool_error_not_a_protocol_error() {
    let h = harness().await;
    h.fake.reply(
        "GET",
        &route("roles"),
        403,
        json!({"code": "E_FORBIDDEN", "status": 403, "message": "Insufficient permissions"}),
    );
    let result = h.call("list_roles", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "FerrisKey API 403: Insufficient permissions");
}

#[tokio::test]
async fn an_unreachable_api_is_a_tool_error() {
    let upstream = Upstream::new("http://127.0.0.1:1/{namespace}/{name}", "").unwrap();
    let mcp = serve(router(upstream)).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(mcp_url(&mcp, NS, INSTANCE))
            .auth_header(jwt("https://x/realms/acme")),
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
    let result = client
        .call_tool(CallToolRequestParams::new("list_roles"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "the FerrisKey API is unreachable");
}

#[tokio::test]
async fn the_instance_names_the_service_and_nothing_else() {
    let h = harness_for("other-ns", "beta", jwt("https://x/realms/acme")).await;
    h.fake.reply(
        "GET",
        "/i/other-ns/beta/api/realms/acme/roles",
        200,
        json!({"data": []}),
    );
    let result = h.call("list_roles", json!({})).await;
    assert_eq!(data(&result), json!({"data": []}));
    assert_eq!(h.fake.only().path, "/i/other-ns/beta/api/realms/acme/roles");
}

#[tokio::test]
async fn a_name_or_namespace_that_is_not_a_dns_label_is_a_404_and_calls_nothing() {
    let h = harness().await;
    let http = reqwest::Client::new();
    let long = "a".repeat(64);
    let bad = [
        ("Tenant", "acme"),
        ("tenant", "Acme"),
        ("-tenant", "acme"),
        ("tenant", "acme-"),
        ("ten_ant", "acme"),
        ("tenant", "a.b"),
        ("tenant.evil.com", "acme"),
        ("..", "acme"),
        ("tenant", ".."),
        ("%2e%2e", "acme"),
        ("tenant", "%2e%2e"),
        ("tenant%2f..", "acme"),
        ("tenant", "acme%2f..%2fx"),
        ("tenant", "acme%00"),
        ("tenant", "acme%0d%0aHost:x"),
        ("tenant@evil.com", "acme"),
        ("tenant", "acme:3333"),
        ("tenant", "acme%3f"),
        ("tenant", "acme%23"),
        (long.as_str(), "acme"),
        ("tenant", long.as_str()),
    ];
    for (namespace, name) in bad {
        let response = http
            .post(mcp_url(&h.mcp, namespace, name))
            .bearer_auth(&h.token)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{namespace} / {name}");
    }
    let outside = http
        .post(format!("{}/instances/tenant/acme/extra/mcp", h.mcp))
        .bearer_auth(&h.token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(outside.status(), 404);
    let bare = http
        .post(format!("{}/mcp", h.mcp))
        .bearer_auth(&h.token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), 404);
    assert!(h.fake.seen().is_empty());
}

#[tokio::test]
async fn a_request_without_a_bearer_is_refused_and_health_is_open() {
    let h = harness().await;
    let http = reqwest::Client::new();
    let refused = http
        .post(mcp_url(&h.mcp, NS, INSTANCE))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
    assert_eq!(refused.headers()["www-authenticate"], "Bearer");
    let health = http.get(format!("{}/healthz", h.mcp)).send().await.unwrap();
    assert_eq!(health.status(), 200);
}

#[test]
fn the_template_must_name_both_the_namespace_and_the_instance() {
    assert!(Upstream::new("http://{name}-api.svc:3333", "/api").is_err());
    assert!(Upstream::new("http://api.{namespace}.svc:3333", "/api").is_err());
    assert!(Upstream::new("http://{name}-api.{namespace}.svc:3333", "/api").is_ok());
}

#[test]
fn the_template_must_be_an_http_url_without_credentials_query_or_fragment() {
    for template in [
        "ftp://{name}.{namespace}.svc",
        "{name}.{namespace}.svc",
        "http://user:pw@{name}.{namespace}.svc",
        "http://{name}.{namespace}.svc/?x=1",
        "http://{name}.{namespace}.svc/#f",
        "http://{name}.{namespace}.svc/{name}/../..",
    ] {
        assert!(Upstream::new(template, "/api").is_err(), "{template}");
    }
}

#[test]
fn the_root_path_is_plain_segments() {
    for root in ["", "/", "/api", "api", "/auth/api/"] {
        assert!(
            Upstream::new("http://{name}.{namespace}.svc", root).is_ok(),
            "{root}"
        );
    }
    for root in ["/..", "/a/../b", "/a?b", "/a#b", "/a b", "/a%2fb"] {
        assert!(
            Upstream::new("http://{name}.{namespace}.svc", root).is_err(),
            "{root}"
        );
    }
}
