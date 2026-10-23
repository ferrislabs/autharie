mod keys;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Form, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mcp_gateway::{
    app::router,
    build_state,
    config::GatewayConfig,
    decision::{DecisionRecord, DecisionSink},
};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::net::TcpListener;

const RESOURCE: &str = "https://auth.acme.test/mcp/crm";
const HOST: &str = "auth.acme.test";
const META_PROTOCOL: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

async fn serve(app: Router) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[derive(Default)]
struct AsState {
    exchanges: AtomicUsize,
}

async fn jwks() -> Json<Value> {
    Json(json!({"keys": [{
        "kty": "RSA", "alg": "RS256", "use": "sig",
        "kid": keys::KID, "n": keys::PUBLIC_N, "e": keys::PUBLIC_E,
    }]}))
}

async fn token_endpoint(
    State(state): State<Arc<AsState>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    match form.get("grant_type").map(String::as_str) {
        Some("client_credentials") => {
            let token = match form.get("client_id").map(String::as_str) {
                Some("pdp-client") => "pdp-token",
                _ => "actor-token",
            };
            Json(json!({"access_token": token, "expires_in": 300})).into_response()
        }
        Some("urn:ietf:params:oauth:grant-type:token-exchange") => {
            let n = state.exchanges.fetch_add(1, Ordering::SeqCst) + 1;
            Json(json!({
                "access_token": format!("delegated-{n}"),
                "issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
                "token_type": "Bearer",
                "expires_in": 300,
            }))
            .into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[derive(Default)]
struct PdpState {
    allowed: Mutex<HashSet<String>>,
}

impl PdpState {
    fn decide(&self, id: &str) -> Value {
        let allowed = self.allowed.lock().unwrap().contains(id);
        json!({"decision": allowed, "context": {"reason_admin": {"policies": ["p"]}}})
    }
}

async fn evaluation(State(state): State<Arc<PdpState>>, Json(body): Json<Value>) -> Response {
    Json(state.decide(body["resource"]["id"].as_str().unwrap_or_default())).into_response()
}

async fn evaluations(State(state): State<Arc<PdpState>>, Json(body): Json<Value>) -> Response {
    let items: Vec<Value> = body["evaluations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| state.decide(item["resource"]["id"].as_str().unwrap_or_default()))
        .collect();
    Json(json!({"evaluations": items})).into_response()
}

#[derive(Clone, Debug)]
struct Seen {
    authorization: Option<String>,
    session: Option<String>,
    protocol: Option<String>,
    mcp_method: Option<String>,
    mcp_name: Option<String>,
    body: Value,
}

fn seen(headers: &HeaderMap, body: &Value) -> Seen {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    Seen {
        authorization: header("authorization"),
        session: header("mcp-session-id"),
        protocol: header("mcp-protocol-version"),
        mcp_method: header("mcp-method"),
        mcp_name: header("mcp-name"),
        body: body.clone(),
    }
}

fn catalogue() -> Value {
    json!([
        {"name": "read_a", "annotations": {"readOnlyHint": true}, "inputSchema": {"type": "object"}},
        {"name": "write_b", "inputSchema": {"type": "object"}},
    ])
}

fn event(message: &Value) -> Response {
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        format!("event: message\ndata: {message}\n\n"),
    )
        .into_response()
}

#[derive(Default)]
struct Legacy {
    seen: Mutex<Vec<Seen>>,
    sessions: Mutex<HashSet<String>>,
    opened: AtomicUsize,
    always_gone: AtomicBool,
}

async fn legacy_upstream(
    State(state): State<Arc<Legacy>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.seen.lock().unwrap().push(seen(&headers, &body));
    let id = body["id"].clone();
    match body["method"].as_str() {
        Some("initialize") => {
            let n = state.opened.fetch_add(1, Ordering::SeqCst) + 1;
            let session = format!("sess-{n}");
            state.sessions.lock().unwrap().insert(session.clone());
            let mut response = event(&json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}},
            }));
            response
                .headers_mut()
                .insert("mcp-session-id", session.parse().unwrap());
            return response;
        }
        Some("notifications/initialized") => return StatusCode::ACCEPTED.into_response(),
        _ => {}
    }
    let known = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| state.sessions.lock().unwrap().contains(s));
    if !known || state.always_gone.load(Ordering::SeqCst) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match body["method"].as_str() {
        Some("tools/list") => {
            Json(json!({"jsonrpc": "2.0", "id": id, "result": {"tools": catalogue()}}))
                .into_response()
        }
        _ => {
            let progress =
                json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {}});
            let result = json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"content": [{"type": "text", "text": "legacy result"}]},
            });
            (
                [(header::CONTENT_TYPE, "text/event-stream")],
                format!("data: {progress}\n\nevent: message\ndata: {result}\n\n"),
            )
                .into_response()
        }
    }
}

#[derive(Default)]
struct Modern {
    seen: Mutex<Vec<Seen>>,
}

async fn modern_upstream(
    State(state): State<Arc<Modern>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let recorded = seen(&headers, &body);
    let complete = recorded.mcp_method.is_some()
        && recorded.protocol.as_deref() == Some("2026-07-28")
        && body["params"]["_meta"][META_PROTOCOL] == "2026-07-28"
        && body["params"]["_meta"][META_CAPABILITIES].is_object();
    state.seen.lock().unwrap().push(recorded);
    if !complete {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"jsonrpc": "2.0", "id": body["id"], "error": {"code": -32600, "message": "not a 2026 request"}})),
        )
            .into_response();
    }
    let id = body["id"].clone();
    match body["method"].as_str() {
        Some("tools/list") => {
            Json(json!({"jsonrpc": "2.0", "id": id, "result": {"tools": catalogue()}}))
                .into_response()
        }
        _ => Json(json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"content": [{"type": "text", "text": "modern result"}]},
        }))
        .into_response(),
    }
}

#[derive(Clone, Default)]
struct Collector(Arc<Mutex<Vec<DecisionRecord>>>);

impl DecisionSink for Collector {
    fn record(&self, record: &DecisionRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

struct World {
    gateway: String,
    issuer: String,
    auth: Arc<AsState>,
    pdp: Arc<PdpState>,
    decisions: Collector,
    http: Client,
}

impl World {
    async fn start(upstream: Router, protocol: Option<&str>) -> Self {
        let upstream_port = serve(upstream).await;
        let auth = Arc::new(AsState::default());
        let issuer_port = serve(
            Router::new()
                .route("/realms/r/protocol/openid-connect/certs", get(jwks))
                .route(
                    "/realms/r/protocol/openid-connect/token",
                    post(token_endpoint),
                )
                .with_state(Arc::clone(&auth)),
        )
        .await;
        let issuer = format!("http://127.0.0.1:{issuer_port}/realms/r");
        let pdp = Arc::new(PdpState::default());
        let pdp_port = serve(
            Router::new()
                .route("/access/v1/evaluation", post(evaluation))
                .route("/access/v1/evaluations", post(evaluations))
                .with_state(Arc::clone(&pdp)),
        )
        .await;
        let protocol = protocol
            .map(|p| format!("protocol = \"{p}\""))
            .unwrap_or_default();
        let source = format!(
            r#"
            [[upstreams]]
            canonical_uri = "{RESOURCE}"
            issuer = "{issuer}"
            upstream_url = "http://127.0.0.1:{upstream_port}/mcp"
            server_name = "crm"
            {protocol}

            [upstreams.pdp]
            evaluation_url = "http://127.0.0.1:{pdp_port}/access/v1/evaluation"
            evaluations_url = "http://127.0.0.1:{pdp_port}/access/v1/evaluations"
            client_id = "pdp-client"
            client_secret = "pdp-secret"

            [upstreams.exchange]
            token_url = "{issuer}/protocol/openid-connect/token"
            client_id = "mcp-gateway"
            client_secret = "gw-secret"
            audience = "mcp-crm"
            "#
        );
        let decisions = Collector::default();
        let state = build_state(
            GatewayConfig::from_toml(&source).unwrap(),
            decisions.clone(),
        )
        .unwrap();
        let gateway_port = serve(router(state)).await;
        Self {
            gateway: format!("http://127.0.0.1:{gateway_port}"),
            issuer,
            auth,
            pdp,
            decisions,
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    async fn legacy(protocol: &str) -> (Self, Arc<Legacy>) {
        let state = Arc::new(Legacy::default());
        let app = Router::new()
            .route("/mcp", post(legacy_upstream))
            .with_state(Arc::clone(&state));
        (Self::start(app, Some(protocol)).await, state)
    }

    async fn modern() -> (Self, Arc<Modern>) {
        let state = Arc::new(Modern::default());
        let app = Router::new()
            .route("/mcp", post(modern_upstream))
            .with_state(Arc::clone(&state));
        (Self::start(app, None).await, state)
    }

    fn allow(&self, resources: &[&str]) {
        self.pdp
            .allowed
            .lock()
            .unwrap()
            .extend(resources.iter().map(|r| r.to_string()));
    }

    fn token(&self, sub: &str) -> String {
        let claims = json!({
            "iss": self.issuer, "aud": RESOURCE, "sub": sub, "azp": "claude",
            "scope": "mcp.tools", "exp": now() + 600,
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(keys::KID.into());
        encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(keys::PRIVATE_KEY_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn post(&self, token: Option<&str>, body: &Value) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(format!("{}/mcp/crm", self.gateway))
            .header("host", HOST)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request
    }

    async fn call_2026(&self, token: &str, id: u64, tool: &str) -> reqwest::Response {
        self.post(
            Some(token),
            &json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {
                    "name": tool, "arguments": {},
                    "_meta": {
                        META_PROTOCOL: "2026-07-28",
                        META_CAPABILITIES: {},
                        "progressToken": "p1",
                    },
                },
            }),
        )
        .header("mcp-method", "tools/call")
        .header("mcp-name", tool)
        .header("mcp-protocol-version", "2026-07-28")
        .send()
        .await
        .unwrap()
    }

    async fn rpc(&self, token: &str, body: Value) -> reqwest::Response {
        self.post(Some(token), &body).send().await.unwrap()
    }
}

fn methods(seen: &[Seen]) -> Vec<String> {
    seen.iter()
        .map(|s| s.body["method"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn spec_ms_9_a_2026_call_reaches_a_2025_sse_server_through_a_session_that_is_reused() {
    let (w, upstream) = World::legacy("2025-11-25").await;
    w.allow(&["crm/read_a"]);
    let token = w.token("alice");

    let first = w.call_2026(&token, 1, "read_a").await;

    assert_eq!(first.status(), 200);
    assert!(
        first.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let answer: Value = first.json().await.unwrap();
    assert_eq!(answer["id"], 1);
    assert_eq!(answer["result"]["content"][0]["text"], "legacy result");

    let second = w.call_2026(&token, 2, "read_a").await;
    assert_eq!(second.status(), 200);
    assert_eq!(second.json::<Value>().await.unwrap()["id"], 2);

    let seen = upstream.seen.lock().unwrap().clone();
    assert_eq!(
        methods(&seen),
        [
            "initialize",
            "notifications/initialized",
            "tools/call",
            "tools/call"
        ]
    );
    assert_eq!(upstream.opened.load(Ordering::SeqCst), 1);
    let init = &seen[0].body["params"];
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert_eq!(init["capabilities"], json!({}));
    assert_eq!(init["clientInfo"]["name"], "autharie-mcp-gateway");
    assert_eq!(seen[0].session, None);
    for call in &seen[1..] {
        assert_eq!(call.session.as_deref(), Some("sess-1"));
        assert_eq!(call.protocol.as_deref(), Some("2025-11-25"));
        assert_eq!(call.authorization.as_deref(), Some("Bearer delegated-1"));
        assert_eq!(call.mcp_method, None);
    }
    let meta = &seen[2].body["params"]["_meta"];
    assert_eq!(meta, &json!({"progressToken": "p1"}));
    assert_eq!(w.auth.exchanges.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn spec_ms_9_each_subject_gets_its_own_session_and_a_denied_call_opens_none() {
    let (w, upstream) = World::legacy("2025-11-25").await;
    w.allow(&["crm/read_a"]);

    let denied = w.call_2026(&w.token("alice"), 1, "write_b").await;
    assert_eq!(denied.status(), 403);
    assert!(upstream.seen.lock().unwrap().is_empty());

    assert_eq!(
        w.call_2026(&w.token("alice"), 2, "read_a").await.status(),
        200
    );
    assert_eq!(
        w.call_2026(&w.token("bob"), 3, "read_a").await.status(),
        200
    );
    assert_eq!(upstream.opened.load(Ordering::SeqCst), 2);
    let records = w.decisions.0.lock().unwrap().clone();
    assert_eq!(records[0].decision, "deny");
    assert_eq!(records[1].decision, "allow");
}

#[tokio::test]
async fn spec_ms_10_a_forgotten_session_is_reopened_once_and_the_call_succeeds() {
    let (w, upstream) = World::legacy("2025-11-25").await;
    w.allow(&["crm/read_a"]);
    let token = w.token("alice");
    assert_eq!(w.call_2026(&token, 1, "read_a").await.status(), 200);

    upstream.sessions.lock().unwrap().clear();
    let after = w.call_2026(&token, 2, "read_a").await;

    assert_eq!(after.status(), 200);
    assert_eq!(after.json::<Value>().await.unwrap()["id"], 2);
    assert_eq!(upstream.opened.load(Ordering::SeqCst), 2);
    let seen = upstream.seen.lock().unwrap().clone();
    let last = seen.last().unwrap();
    assert_eq!(last.session.as_deref(), Some("sess-2"));
}

#[tokio::test]
async fn spec_ms_10_a_second_404_is_an_upstream_error_not_a_loop() {
    let (w, upstream) = World::legacy("2025-11-25").await;
    w.allow(&["crm/read_a"]);
    upstream.always_gone.store(true, Ordering::SeqCst);

    let response = w.call_2026(&w.token("alice"), 1, "read_a").await;

    assert_eq!(response.status(), 502);
    assert_eq!(upstream.opened.load(Ordering::SeqCst), 2);
    let calls = methods(&upstream.seen.lock().unwrap())
        .iter()
        .filter(|m| *m == "tools/call")
        .count();
    assert_eq!(calls, 2);
}

#[tokio::test]
async fn spec_ms_9_tools_list_from_a_legacy_server_is_filtered_for_the_caller() {
    let (w, _) = World::legacy("2025-06-18").await;
    w.allow(&["crm/read_a"]);

    let response = w
        .rpc(
            &w.token("alice"),
            json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}),
        )
        .await;

    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let answer: Value = response.json().await.unwrap();
    let names: Vec<&str> = answer["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["read_a"]);
}

#[tokio::test]
async fn spec_ms_11_a_2025_client_walks_the_whole_flow_in_front_of_a_2026_server() {
    let (w, upstream) = World::modern().await;
    w.allow(&["crm/read_a"]);
    let token = w.token("alice");

    let init = w
        .rpc(
            &token,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}},
            }),
        )
        .await;
    assert_eq!(init.status(), 200);
    let session = init.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(!session.is_empty());
    let init: Value = init.json().await.unwrap();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "crm");

    let initialized = w
        .post(
            Some(&token),
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(initialized.status(), 202);
    assert!(initialized.bytes().await.unwrap().is_empty());
    assert!(upstream.seen.lock().unwrap().is_empty());

    let list = w
        .post(
            Some(&token),
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .header("mcp-session-id", &session)
        .header("mcp-protocol-version", "2025-06-18")
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list: Value = list.json().await.unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["read_a"]);

    let call = w
        .post(
            Some(&token),
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "read_a", "arguments": {}}}),
        )
        .header("mcp-session-id", &session)
        .header("mcp-protocol-version", "2025-06-18")
        .send()
        .await
        .unwrap();
    assert_eq!(call.status(), 200);
    let call: Value = call.json().await.unwrap();
    assert_eq!(call["result"]["content"][0]["text"], "modern result");

    let seen = upstream.seen.lock().unwrap().clone();
    assert_eq!(methods(&seen), ["tools/list", "tools/call"]);
    assert_eq!(seen[0].mcp_method.as_deref(), Some("tools/list"));
    assert_eq!(seen[1].mcp_method.as_deref(), Some("tools/call"));
    assert_eq!(seen[1].mcp_name.as_deref(), Some("read_a"));
    assert!(seen.iter().all(|s| s.session.is_none()));
    assert!(
        seen.iter()
            .all(|s| s.protocol.as_deref() == Some("2026-07-28"))
    );
}

#[tokio::test]
async fn spec_ms_11_a_header_that_disagrees_with_the_body_is_still_refused() {
    let (w, upstream) = World::modern().await;
    w.allow(&["crm/read_a"]);

    let response = w
        .post(
            Some(&w.token("alice")),
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "read_a"}}),
        )
        .header("mcp-method", "tools/call")
        .header("mcp-name", "write_b")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        -32020
    );
    assert!(upstream.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spec_ms_11_initialize_still_needs_a_valid_token() {
    let (w, upstream) = World::modern().await;
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});

    let anonymous = w.post(None, &body).send().await.unwrap();
    assert_eq!(anonymous.status(), 401);
    assert!(
        anonymous.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .contains("resource_metadata=")
    );
    let forged = w.post(Some("not-a-jwt"), &body).send().await.unwrap();
    assert_eq!(forged.status(), 401);
    assert!(upstream.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spec_ms_11_an_unsupported_client_version_gets_the_newest_2025_revision() {
    let (w, _) = World::modern().await;

    let response = w
        .rpc(
            &w.token("alice"),
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2030-01-01"}}),
        )
        .await;

    let answer: Value = response.json().await.unwrap();
    assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");
}
