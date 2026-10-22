mod keys;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
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
use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};
use tokio::net::TcpListener;

const RESOURCE: &str = "https://mcp.example/mcp";
const OTHER_RESOURCE: &str = "https://other.example/mcp";
const METADATA: &str = "https://mcp.example/.well-known/oauth-protected-resource/mcp";

async fn serve(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
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
    fail_exchange: Mutex<bool>,
    forms: Mutex<Vec<HashMap<String, String>>>,
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
    state.forms.lock().unwrap().push(form.clone());
    match form.get("grant_type").map(String::as_str) {
        Some("client_credentials") => {
            let token = match form.get("client_id").map(String::as_str) {
                Some("pdp-client") => "pdp-token",
                _ => "actor-token",
            };
            Json(json!({"access_token": token, "expires_in": 300})).into_response()
        }
        Some("urn:ietf:params:oauth:grant-type:token-exchange") => {
            if *state.fail_exchange.lock().unwrap() {
                return StatusCode::BAD_REQUEST.into_response();
            }
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

#[derive(Clone, Copy, PartialEq)]
enum PdpMode {
    Working,
    Broken,
    Slow,
    BatchMissing,
}

struct PdpState {
    mode: Mutex<PdpMode>,
    allowed: Mutex<HashSet<String>>,
    singles: Mutex<Vec<Value>>,
    batches: Mutex<Vec<Value>>,
}

impl PdpState {
    fn decide(&self, resource_id: &str) -> Value {
        let allowed = self.allowed.lock().unwrap().contains(resource_id);
        json!({
            "decision": allowed,
            "context": {"reason_admin": {"policies": [format!("policy-for-{resource_id}")]}},
        })
    }

    async fn gate(&self, headers: &HeaderMap) -> Option<Response> {
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some("Bearer pdp-token")
        {
            return Some(StatusCode::UNAUTHORIZED.into_response());
        }
        let mode = *self.mode.lock().unwrap();
        match mode {
            PdpMode::Broken => return Some(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            PdpMode::Slow => {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            PdpMode::Working | PdpMode::BatchMissing => {}
        }
        None
    }
}

async fn evaluation(
    State(state): State<Arc<PdpState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.singles.lock().unwrap().push(body.clone());
    if let Some(refusal) = state.gate(&headers).await {
        return refusal;
    }
    let id = body["resource"]["id"].as_str().unwrap_or_default();
    Json(state.decide(id)).into_response()
}

async fn evaluations(
    State(state): State<Arc<PdpState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.batches.lock().unwrap().push(body.clone());
    if *state.mode.lock().unwrap() == PdpMode::BatchMissing {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(refusal) = state.gate(&headers).await {
        return refusal;
    }
    let items: Vec<Value> = body["evaluations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| state.decide(item["resource"]["id"].as_str().unwrap_or_default()))
        .collect();
    Json(json!({"evaluations": items})).into_response()
}

#[derive(Default)]
struct UpstreamState {
    calls: Mutex<Vec<Recorded>>,
    sse: Mutex<bool>,
}

#[derive(Clone, Debug)]
struct Recorded {
    authorization: Option<String>,
    request_id: Option<String>,
    body: Value,
}

fn catalogue() -> Vec<Value> {
    vec![
        json!({"name": "read_a", "annotations": {"readOnlyHint": true}, "inputSchema": {"type": "object"}}),
        json!({"name": "write_b", "annotations": {"readOnlyHint": false}, "inputSchema": {"type": "object"}}),
        json!({"name": "secret_c", "inputSchema": {"type": "object"}}),
    ]
}

async fn upstream(
    State(state): State<Arc<UpstreamState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    state.calls.lock().unwrap().push(Recorded {
        authorization: header("authorization"),
        request_id: header("x-request-id"),
        body: body.clone(),
    });
    let id = body["id"].clone();
    let message = match body["method"].as_str() {
        Some("tools/list") => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": catalogue()}}),
        Some("tools/call") => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"content": [{"type": "text", "text": format!("ran {}", body["params"]["name"])}]},
        }),
        _ => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
    };
    if *state.sse.lock().unwrap() && body["method"] == "tools/list" {
        let text = format!("event: message\nid: 1\ndata: {message}\n\n");
        return ([(header::CONTENT_TYPE, "text/event-stream")], text).into_response();
    }
    Json(message).into_response()
}

#[derive(Clone, Default)]
struct Collector(Arc<Mutex<Vec<DecisionRecord>>>);

impl DecisionSink for Collector {
    fn record(&self, record: &DecisionRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

struct Harness {
    gateway: String,
    issuer: String,
    http: Client,
    auth: Arc<AsState>,
    pdp: Arc<PdpState>,
    upstream: Arc<UpstreamState>,
    other_upstream: Arc<UpstreamState>,
    decisions: Collector,
}

struct Options {
    pdp_timeout_ms: u64,
    pdp_unreachable: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pdp_timeout_ms: 1000,
            pdp_unreachable: false,
        }
    }
}

impl Harness {
    async fn start() -> Self {
        Self::start_with(Options::default()).await
    }

    async fn start_with(options: Options) -> Self {
        let auth = Arc::new(AsState::default());
        let issuer_base = serve(
            Router::new()
                .route("/realms/r/protocol/openid-connect/certs", get(jwks))
                .route(
                    "/realms/r/protocol/openid-connect/token",
                    post(token_endpoint),
                )
                .with_state(Arc::clone(&auth)),
        )
        .await;
        let issuer = format!("{issuer_base}/realms/r");

        let pdp = Arc::new(PdpState {
            mode: Mutex::new(PdpMode::Working),
            allowed: Mutex::default(),
            singles: Mutex::default(),
            batches: Mutex::default(),
        });
        let pdp_base = if options.pdp_unreachable {
            let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", closed.local_addr().unwrap())
        } else {
            serve(
                Router::new()
                    .route("/access/v1/evaluation", post(evaluation))
                    .route("/access/v1/evaluations", post(evaluations))
                    .with_state(Arc::clone(&pdp)),
            )
            .await
        };

        let up = Arc::new(UpstreamState::default());
        let upstream_base = serve(
            Router::new()
                .route("/mcp", post(upstream))
                .with_state(Arc::clone(&up)),
        )
        .await;
        let other_up = Arc::new(UpstreamState::default());
        let other_base = serve(
            Router::new()
                .route("/mcp", post(upstream))
                .with_state(Arc::clone(&other_up)),
        )
        .await;

        let section = |canonical: &str, upstream_url: &str, server: &str, audience: &str| {
            format!(
                r#"
                [[upstreams]]
                canonical_uri = "{canonical}"
                issuer = "{issuer}"
                upstream_url = "{upstream_url}/mcp"
                server_name = "{server}"
                scopes_supported = ["mcp.tools"]

                [upstreams.pdp]
                evaluation_url = "{pdp_base}/access/v1/evaluation"
                evaluations_url = "{pdp_base}/access/v1/evaluations"
                client_id = "pdp-client"
                client_secret = "pdp-secret"

                [upstreams.exchange]
                token_url = "{issuer}/protocol/openid-connect/token"
                client_id = "gateway"
                client_secret = "gw-secret"
                audience = "{audience}"
                "#
            )
        };
        let source = format!(
            "pdp_timeout_ms = {}\n{}{}",
            options.pdp_timeout_ms,
            section(RESOURCE, &upstream_base, "autharie", "autharie-api"),
            section(OTHER_RESOURCE, &other_base, "ferriskey", "ferriskey-api"),
        );
        let config = GatewayConfig::from_toml(&source).unwrap();
        let decisions = Collector::default();
        let gateway = serve(router(build_state(config, decisions.clone()).unwrap())).await;
        Self {
            gateway,
            issuer,
            http: Client::new(),
            auth,
            pdp,
            upstream: up,
            other_upstream: other_up,
            decisions,
        }
    }

    fn allow(&self, ids: &[&str]) {
        *self.pdp.allowed.lock().unwrap() = ids.iter().map(|s| (*s).to_owned()).collect();
    }

    fn token(&self, overrides: Value) -> String {
        let mut claims = json!({
            "iss": self.issuer,
            "aud": RESOURCE,
            "sub": "alice",
            "azp": "claude",
            "scope": "mcp.tools",
            "sid": "session-1",
            "roles": ["admin"],
            "groups": ["support"],
            "organisation": "acme",
            "exp": now() + 600,
        });
        for (k, v) in overrides.as_object().unwrap() {
            if v.is_null() {
                claims.as_object_mut().unwrap().remove(k);
            } else {
                claims[k] = v.clone();
            }
        }
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(keys::KID.into());
        encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(keys::PRIVATE_KEY_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn post(&self, host: &str, token: Option<&str>, body: &Value) -> RequestBuilder {
        let method = body["method"].as_str();
        let name = body["params"]["name"].as_str();
        self.post_with_headers(host, token, body, method, name)
    }

    fn post_with_headers(
        &self,
        host: &str,
        token: Option<&str>,
        body: &Value,
        method: Option<&str>,
        name: Option<&str>,
    ) -> RequestBuilder {
        let mut request = self
            .http
            .post(format!("{}/mcp", self.gateway))
            .header("host", host)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(body);
        if let Some(method) = method {
            request = request.header("mcp-method", method);
        }
        if let Some(name) = name {
            request = request.header("mcp-name", name);
        }
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request
    }

    async fn send(&self, token: &str, body: Value) -> reqwest::Response {
        self.post("mcp.example", Some(token), &body)
            .send()
            .await
            .unwrap()
    }

    fn records(&self) -> Vec<DecisionRecord> {
        self.decisions.0.lock().unwrap().clone()
    }

    fn upstream_calls(&self) -> Vec<Recorded> {
        self.upstream.calls.lock().unwrap().clone()
    }
}

fn call(name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

fn list() -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
}

fn tool_names(body: &Value) -> Vec<String> {
    body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn health_answers_on_any_host() {
    let h = Harness::start().await;
    let response = h
        .http
        .get(format!("{}/healthz", h.gateway))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn spec_ag_1_no_token_gets_a_challenge_naming_metadata_and_scope() {
    let h = Harness::start().await;
    let response = h.post("mcp.example", None, &list()).send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.headers()["www-authenticate"],
        format!("Bearer resource_metadata=\"{METADATA}\", scope=\"mcp.tools\"")
    );
}

#[tokio::test]
async fn spec_ag_1_metadata_is_served_per_upstream() {
    let h = Harness::start().await;
    let fetch = |host: &'static str, path: &'static str| {
        h.http
            .get(format!("{}{path}", h.gateway))
            .header("host", host)
            .send()
    };
    let body: Value = fetch("mcp.example", "/.well-known/oauth-protected-resource/mcp")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["resource"], RESOURCE);
    assert_eq!(body["authorization_servers"], json!([h.issuer]));
    assert_eq!(body["scopes_supported"], json!(["mcp.tools"]));
    let other: Value = fetch("other.example", "/.well-known/oauth-protected-resource/mcp")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(other["resource"], OTHER_RESOURCE);
    let unknown = fetch(
        "nobody.example",
        "/.well-known/oauth-protected-resource/mcp",
    )
    .await
    .unwrap();
    assert_eq!(unknown.status(), 404);
}

#[tokio::test]
async fn spec_ag_4_a_token_for_another_resource_is_refused() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);

    let wrong_audience = h.token(json!({"aud": "https://elsewhere.example/mcp"}));
    let response = h.send(&wrong_audience, call("read_a", json!({}))).await;
    assert_eq!(response.status(), 401);
    assert!(
        response.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .starts_with("Bearer error=\"invalid_token\", resource_metadata=")
    );

    let for_other_upstream = h.token(json!({"aud": OTHER_RESOURCE}));
    assert_eq!(
        h.send(&for_other_upstream, call("read_a", json!({})))
            .await
            .status(),
        401
    );

    let wrong_issuer = h.token(json!({"iss": "https://evil.example/realms/r"}));
    assert_eq!(
        h.send(&wrong_issuer, call("read_a", json!({})))
            .await
            .status(),
        401
    );

    let expired = h.token(json!({"exp": now() - 600}));
    assert_eq!(
        h.send(&expired, call("read_a", json!({}))).await.status(),
        401
    );

    assert!(h.upstream_calls().is_empty());
    assert!(h.pdp.singles.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spec_ag_2_an_allowed_call_reaches_the_upstream_with_an_exchanged_token() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    let token = h.token(json!({}));

    let response = h
        .send(
            &token,
            call(
                "read_a",
                json!({"organisation": "acme", "limit": 5, "nested": {"x": 1}}),
            ),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["result"]["content"][0]["text"], "ran \"read_a\"");

    let calls = h.upstream_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].authorization.as_deref(),
        Some("Bearer delegated-1")
    );
    assert_ne!(
        calls[0].authorization.as_deref(),
        Some(&*format!("Bearer {token}"))
    );
    assert!(calls[0].request_id.is_some());
    assert_eq!(calls[0].body["params"]["name"], "read_a");

    let forms = h.auth.forms.lock().unwrap().clone();
    let exchange = forms
        .iter()
        .find(|f| {
            f.get("grant_type")
                .is_some_and(|g| g.contains("token-exchange"))
        })
        .unwrap();
    assert_eq!(exchange["subject_token"], token);
    assert_eq!(exchange["audience"], "autharie-api");
    assert_eq!(exchange["actor_token"], "actor-token");
    assert_eq!(exchange["client_id"], "gateway");

    let singles = h.pdp.singles.lock().unwrap().clone();
    assert_eq!(singles.len(), 1);
    let request = &singles[0];
    assert_eq!(
        request["subject"],
        json!({"type": "User", "id": "alice", "properties": {
            "roles": ["admin"], "groups": ["support"], "organisation": "acme"}})
    );
    assert_eq!(request["action"], json!({"name": "call_tool"}));
    assert_eq!(request["resource"]["type"], "Tool");
    assert_eq!(request["resource"]["id"], "autharie/read_a");
    assert_eq!(request["resource"]["properties"]["server"], "autharie");
    assert_eq!(request["resource"]["properties"]["tool"], "read_a");
    assert_eq!(request["context"]["agent"], "claude");
    assert_eq!(request["context"]["delegated"], true);
    assert_eq!(
        request["context"]["arguments"],
        json!({"organisation": "acme", "limit": 5})
    );
    let time = &request["context"]["time"];
    assert!(time["iso"].as_str().unwrap().ends_with('Z'));
    assert!((1..=7).contains(&time["weekday"].as_u64().unwrap()));
    assert!(time["hour"].as_u64().unwrap() < 24);
}

#[tokio::test]
async fn spec_ag_2_an_autonomous_agent_is_a_client_subject() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    let token = h.token(json!({"sid": null, "sub": "service-account-claude"}));

    let response = h.send(&token, call("read_a", json!({}))).await;
    assert_eq!(response.status(), 200);

    let singles = h.pdp.singles.lock().unwrap().clone();
    assert_eq!(
        singles[0]["subject"],
        json!({"type": "Client", "id": "claude"})
    );
    assert_eq!(singles[0]["context"]["delegated"], false);
    let record = &h.records()[0];
    assert_eq!(record.subject_type, "Client");
    assert_eq!(record.subject_id, "claude");
    assert!(!record.delegated);
}

#[tokio::test]
async fn spec_ag_3_a_call_without_a_permitting_policy_is_denied_and_recorded() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    let token = h.token(json!({}));

    let response = h.send(&token, call("write_b", json!({}))).await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32001);
    assert_eq!(body["id"], 1);

    assert!(h.upstream_calls().is_empty());
    let records = h.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.kind, "agent_decision");
    assert_eq!(record.decision, "deny");
    assert_eq!(record.server, "autharie");
    assert_eq!(record.method, "tools/call");
    assert_eq!(record.tool.as_deref(), Some("write_b"));
    assert_eq!(record.subject_id, "alice");
    assert_eq!(record.agent, "claude");
    assert_eq!(record.policies, vec!["policy-for-autharie/write_b"]);
    assert!(!record.request_id.is_empty());
}

#[tokio::test]
async fn every_allowed_call_is_recorded_with_the_deciding_policy() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    h.send(&h.token(json!({})), call("read_a", json!({}))).await;

    let records = h.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].decision, "allow");
    assert_eq!(records[0].policies, vec!["policy-for-autharie/read_a"]);
    assert_eq!(records[0].subject_type, "User");
    assert!(records[0].delegated);
    assert_eq!(records[0].reason, None);
}

#[tokio::test]
async fn spec_ag_5_a_mismatched_mcp_name_is_refused_before_evaluation() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a", "autharie/write_b"]);
    let token = h.token(json!({}));

    let body = call("write_b", json!({}));
    let response = h
        .post_with_headers(
            "mcp.example",
            Some(&token),
            &body,
            Some("tools/call"),
            Some("read_a"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let answer: Value = response.json().await.unwrap();
    assert_eq!(answer["error"]["code"], -32020);

    let wrong_method = h
        .post_with_headers(
            "mcp.example",
            Some(&token),
            &body,
            Some("tools/list"),
            Some("write_b"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_method.status(), 400);

    assert!(h.pdp.singles.lock().unwrap().is_empty());
    assert!(h.pdp.batches.lock().unwrap().is_empty());
    assert!(h.upstream_calls().is_empty());
    let records = h.records();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.decision == "deny"));
    assert_eq!(records[0].reason, Some("header_mismatch"));
}

#[tokio::test]
async fn spec_ag_9_a_broken_pdp_denies_every_call() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    *h.pdp.mode.lock().unwrap() = PdpMode::Broken;

    let response = h.send(&h.token(json!({})), call("read_a", json!({}))).await;
    assert_eq!(response.status(), 403);
    assert!(h.upstream_calls().is_empty());
    let record = &h.records()[0];
    assert_eq!(record.decision, "deny");
    assert_eq!(record.reason, Some("pdp_unavailable"));
}

#[tokio::test]
async fn spec_ag_9_a_pdp_timeout_denies() {
    let h = Harness::start_with(Options {
        pdp_timeout_ms: 200,
        ..Options::default()
    })
    .await;
    h.allow(&["autharie/read_a"]);
    *h.pdp.mode.lock().unwrap() = PdpMode::Slow;

    let response = h.send(&h.token(json!({})), call("read_a", json!({}))).await;
    assert_eq!(response.status(), 403);
    assert!(h.upstream_calls().is_empty());
    assert_eq!(h.records()[0].reason, Some("pdp_timeout"));
}

#[tokio::test]
async fn spec_ag_9_an_unreachable_pdp_denies() {
    let h = Harness::start_with(Options {
        pdp_unreachable: true,
        ..Options::default()
    })
    .await;
    let response = h.send(&h.token(json!({})), call("read_a", json!({}))).await;
    assert_eq!(response.status(), 403);
    assert!(h.upstream_calls().is_empty());
    assert_eq!(h.records()[0].decision, "deny");
}

#[tokio::test]
async fn spec_ag_1_tools_list_shows_only_what_the_caller_may_call() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a", "autharie/secret_c"]);
    let token = h.token(json!({}));

    let response = h.send(&token, list()).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["vary"], "authorization");
    let body: Value = response.json().await.unwrap();
    assert_eq!(tool_names(&body), ["read_a", "secret_c"]);
    assert_eq!(body["result"]["cacheScope"], "private");
    assert_eq!(body["id"], 2);

    let batches = h.pdp.batches.lock().unwrap().clone();
    assert_eq!(batches.len(), 1);
    assert!(h.pdp.singles.lock().unwrap().is_empty());
    let batch = &batches[0];
    assert_eq!(batch["subject"]["type"], "User");
    assert_eq!(batch["action"], json!({"name": "call_tool"}));
    let ids: Vec<&str> = batch["evaluations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["resource"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["autharie/read_a", "autharie/write_b", "autharie/secret_c"]
    );
    assert_eq!(
        batch["evaluations"][0]["resource"]["properties"]["annotations"],
        json!({"readOnlyHint": true})
    );

    let records = h.records();
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|r| r.method == "tools/list"));
    assert_eq!(records.iter().filter(|r| r.decision == "allow").count(), 2);
}

#[tokio::test]
async fn tools_list_is_never_shared_across_callers() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    let alice = h.token(json!({}));
    let bob = h.token(json!({"sub": "bob"}));

    let first: Value = h.send(&alice, list()).await.json().await.unwrap();
    h.allow(&["autharie/write_b"]);
    let second: Value = h.send(&bob, list()).await.json().await.unwrap();
    assert_eq!(tool_names(&first), ["read_a"]);
    assert_eq!(tool_names(&second), ["write_b"]);
    assert_eq!(h.pdp.batches.lock().unwrap().len(), 2);
    assert_eq!(h.upstream_calls().len(), 2);
}

#[tokio::test]
async fn a_pdp_without_batch_falls_back_to_concurrent_single_evaluations() {
    let h = Harness::start().await;
    h.allow(&["autharie/write_b"]);
    *h.pdp.mode.lock().unwrap() = PdpMode::BatchMissing;

    let body: Value = h
        .send(&h.token(json!({})), list())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(tool_names(&body), ["write_b"]);
    assert_eq!(h.pdp.batches.lock().unwrap().len(), 1);
    assert_eq!(h.pdp.singles.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn tools_list_with_a_broken_pdp_is_denied() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    *h.pdp.mode.lock().unwrap() = PdpMode::Broken;

    let response = h.send(&h.token(json!({})), list()).await;
    assert_eq!(response.status(), 403);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let records = h.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].decision, "deny");
    assert_eq!(records[0].tool, None);
}

#[tokio::test]
async fn tools_list_streamed_as_events_is_filtered_too() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    *h.upstream.sse.lock().unwrap() = true;

    let response = h.send(&h.token(json!({})), list()).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let text = response.text().await.unwrap();
    assert!(text.starts_with("event: message\nid: 1\ndata: "));
    assert!(text.contains("read_a"));
    assert!(!text.contains("write_b"));
    assert!(!text.contains("secret_c"));
}

#[tokio::test]
async fn annotations_seen_in_a_list_inform_later_calls() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    let token = h.token(json!({}));

    h.send(&token, list()).await;
    h.send(&token, call("read_a", json!({}))).await;

    let singles = h.pdp.singles.lock().unwrap().clone();
    assert_eq!(
        singles[0]["resource"]["properties"]["annotations"],
        json!({"readOnlyHint": true})
    );
}

#[tokio::test]
async fn the_delegated_token_is_cached_per_subject_and_agent() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    let alice = h.token(json!({}));

    h.send(&alice, call("read_a", json!({}))).await;
    h.send(&alice, call("read_a", json!({}))).await;
    assert_eq!(h.auth.exchanges.load(Ordering::SeqCst), 1);
    let calls = h.upstream_calls();
    assert_eq!(calls[0].authorization, calls[1].authorization);

    let other_agent = h.token(json!({"azp": "cursor"}));
    h.send(&other_agent, call("read_a", json!({}))).await;
    assert_eq!(h.auth.exchanges.load(Ordering::SeqCst), 2);

    let bob = h.token(json!({"sub": "bob"}));
    h.send(&bob, call("read_a", json!({}))).await;
    assert_eq!(h.auth.exchanges.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_failed_exchange_is_a_bad_gateway_and_nothing_reaches_the_upstream() {
    let h = Harness::start().await;
    h.allow(&["autharie/read_a"]);
    *h.auth.fail_exchange.lock().unwrap() = true;

    let response = h.send(&h.token(json!({})), call("read_a", json!({}))).await;
    assert_eq!(response.status(), 502);
    assert!(h.upstream_calls().is_empty());
}

#[tokio::test]
async fn upstreams_are_selected_by_host() {
    let h = Harness::start().await;
    h.pdp
        .allowed
        .lock()
        .unwrap()
        .insert("ferriskey/read_a".into());
    let token = h.token(json!({"aud": OTHER_RESOURCE}));

    let response = h
        .post("other.example", Some(&token), &call("read_a", json!({})))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(h.other_upstream.calls.lock().unwrap().len(), 1);
    assert!(h.upstream_calls().is_empty());
    assert_eq!(h.records()[0].server, "ferriskey");
    let exchange_audiences: Vec<String> = h
        .auth
        .forms
        .lock()
        .unwrap()
        .iter()
        .filter_map(|f| f.get("audience").cloned())
        .collect();
    assert_eq!(exchange_audiences, ["ferriskey-api"]);
}

#[tokio::test]
async fn an_unknown_host_is_not_served() {
    let h = Harness::start().await;
    let response = h
        .post("nobody.example", Some(&h.token(json!({}))), &list())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn methods_that_need_no_decision_are_forwarded_with_the_exchanged_token() {
    let h = Harness::start().await;
    let token = h.token(json!({}));
    let body = json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {}});

    let response = h.send(&token, body).await;
    assert_eq!(response.status(), 200);
    assert!(h.pdp.singles.lock().unwrap().is_empty());
    assert_eq!(
        h.upstream_calls()[0].authorization.as_deref(),
        Some("Bearer delegated-1")
    );
}
