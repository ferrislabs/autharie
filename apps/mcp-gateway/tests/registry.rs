mod keys;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Form, Json, Router,
    extract::State,
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
use std::collections::HashMap;
use tokio::net::TcpListener;

const RESOURCE: &str = "https://mcp.example/mcp";

async fn serve(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn jwks() -> Json<Value> {
    Json(json!({"keys": [{
        "kty": "RSA", "alg": "RS256", "use": "sig",
        "kid": keys::KID, "n": keys::PUBLIC_N, "e": keys::PUBLIC_E,
    }]}))
}

async fn token_endpoint(Form(form): Form<HashMap<String, String>>) -> Response {
    match form.get("grant_type").map(String::as_str) {
        Some("client_credentials") => {
            Json(json!({"access_token": "pdp-token", "expires_in": 300})).into_response()
        }
        _ => Json(json!({
            "access_token": "delegated",
            "issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
            "token_type": "Bearer",
            "expires_in": 300,
        }))
        .into_response(),
    }
}

#[derive(Default)]
struct Counts {
    pdp: AtomicUsize,
    upstream: AtomicUsize,
}

async fn evaluation(State(counts): State<Arc<Counts>>) -> Json<Value> {
    counts.pdp.fetch_add(1, Ordering::SeqCst);
    Json(json!({"decision": true, "context": {"reason_admin": {"policies": ["p1"]}}}))
}

async fn evaluations(State(counts): State<Arc<Counts>>, Json(body): Json<Value>) -> Json<Value> {
    counts.pdp.fetch_add(1, Ordering::SeqCst);
    let items: Vec<Value> = body["evaluations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|_| json!({"decision": true, "context": {"reason_admin": {"policies": ["p1"]}}}))
        .collect();
    Json(json!({"evaluations": items}))
}

async fn upstream(State(counts): State<Arc<Counts>>, Json(body): Json<Value>) -> Response {
    counts.upstream.fetch_add(1, Ordering::SeqCst);
    let id = body["id"].clone();
    let message = match body["method"].as_str() {
        Some("tools/list") => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [
            {"name": "read_a", "inputSchema": {"type": "object"}}
        ]}}),
        _ => json!({"jsonrpc": "2.0", "id": id, "result": {"content": []}}),
    };
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
    counts: Arc<Counts>,
    decisions: Collector,
}

impl Harness {
    async fn start(access: &str) -> Self {
        let issuer_base = serve(
            Router::new()
                .route("/realms/r/protocol/openid-connect/certs", get(jwks))
                .route(
                    "/realms/r/protocol/openid-connect/token",
                    post(token_endpoint),
                ),
        )
        .await;
        let issuer = format!("{issuer_base}/realms/r");
        let counts = Arc::new(Counts::default());
        let backend = serve(
            Router::new()
                .route("/access/v1/evaluation", post(evaluation))
                .route("/access/v1/evaluations", post(evaluations))
                .route("/mcp", post(upstream))
                .with_state(Arc::clone(&counts)),
        )
        .await;
        let source = format!(
            r#"
            [[upstreams]]
            canonical_uri = "{RESOURCE}"
            issuer = "{issuer}"
            upstream_url = "{backend}/mcp"
            server_name = "ferriskey"

            [upstreams.pdp]
            evaluation_url = "{backend}/access/v1/evaluation"
            evaluations_url = "{backend}/access/v1/evaluations"
            client_id = "pdp-client"
            client_secret = "pdp-secret"

            [upstreams.exchange]
            token_url = "{issuer}/protocol/openid-connect/token"
            client_id = "gateway"
            client_secret = "gw-secret"
            audience = "api"

            {access}
            "#
        );
        let decisions = Collector::default();
        let config = GatewayConfig::from_toml(&source).unwrap();
        let gateway = serve(router(build_state(config, decisions.clone()).unwrap())).await;
        Self {
            gateway,
            issuer,
            counts,
            decisions,
        }
    }

    fn token(&self, agent: &str) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = json!({
            "iss": self.issuer,
            "aud": RESOURCE,
            "sub": "alice",
            "azp": agent,
            "sid": "s",
            "exp": now + 600,
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

    async fn send(&self, agent: &str, body: Value) -> reqwest::Response {
        let mut request = Client::new()
            .post(format!("{}/mcp", self.gateway))
            .header("host", "mcp.example")
            .header("mcp-method", body["method"].as_str().unwrap())
            .bearer_auth(self.token(agent))
            .json(&body);
        if let Some(name) = body["params"]["name"].as_str() {
            request = request.header("mcp-name", name);
        }
        request.send().await.unwrap()
    }

    fn records(&self) -> Vec<DecisionRecord> {
        self.decisions.0.lock().unwrap().clone()
    }

    fn pdp_calls(&self) -> usize {
        self.counts.pdp.load(Ordering::SeqCst)
    }

    fn upstream_calls(&self) -> usize {
        self.counts.upstream.load(Ordering::SeqCst)
    }
}

fn list() -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
}

fn call(name: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": {}}})
}

const APPROVAL: &str =
    "[upstreams.access]\nmode = \"approval\"\napproved = [\"trusted\"]\nblocked = [\"banned\"]";

#[tokio::test]
async fn spec_ar_2_a_pending_agent_is_refused_before_the_pdp_and_recorded() {
    let h = Harness::start(APPROVAL).await;

    let response = h.send("newcomer", list()).await;

    assert_eq!(response.status(), 403);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32003);
    assert_eq!(body["id"], 2);
    assert_eq!(h.pdp_calls(), 0);
    assert_eq!(h.upstream_calls(), 0);
    let records = h.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].agent, "newcomer");
    assert_eq!(records[0].decision, "deny");
    assert_eq!(records[0].reason, Some("agent_pending"));
    assert!(records[0].policies.is_empty());
    assert_eq!(records[0].method, "tools/list");
}

#[tokio::test]
async fn spec_ar_2_a_pending_agent_cannot_call_a_tool_either() {
    let h = Harness::start(APPROVAL).await;

    let response = h.send("newcomer", call("read_a")).await;

    assert_eq!(response.status(), 403);
    assert_eq!(h.pdp_calls(), 0);
    let records = h.records();
    assert_eq!(records[0].tool.as_deref(), Some("read_a"));
    assert_eq!(records[0].reason, Some("agent_pending"));
}

#[tokio::test]
async fn spec_ar_2_an_approved_agent_reaches_the_pdp() {
    let h = Harness::start(APPROVAL).await;

    let response = h.send("trusted", call("read_a")).await;

    assert_eq!(response.status(), 200);
    assert_eq!(h.pdp_calls(), 1);
    assert_eq!(h.upstream_calls(), 1);
    assert_eq!(h.records()[0].decision, "allow");
}

#[tokio::test]
async fn spec_ar_3_a_blocked_agent_is_refused_before_the_pdp_whatever_the_policies() {
    let h = Harness::start(APPROVAL).await;

    let response = h.send("banned", call("read_a")).await;

    assert_eq!(response.status(), 403);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32003);
    assert_eq!(h.pdp_calls(), 0);
    assert_eq!(h.upstream_calls(), 0);
    assert_eq!(h.records()[0].reason, Some("agent_blocked"));
}

#[tokio::test]
async fn spec_ar_3_a_blocked_agent_is_refused_even_when_approval_mode_is_off() {
    let h = Harness::start("[upstreams.access]\nblocked = [\"banned\"]").await;

    let response = h.send("banned", list()).await;

    assert_eq!(response.status(), 403);
    assert_eq!(h.pdp_calls(), 0);
    assert_eq!(h.records()[0].reason, Some("agent_blocked"));
}

#[tokio::test]
async fn spec_ar_4_with_approval_mode_off_an_unknown_agent_reaches_the_pdp() {
    let h = Harness::start("").await;

    let response = h.send("newcomer", call("read_a")).await;

    assert_eq!(response.status(), 200);
    assert_eq!(h.pdp_calls(), 1);
    assert_eq!(h.records()[0].reason, None);
}
