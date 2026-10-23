mod keys;

use std::{
    collections::HashMap,
    io,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use autharie_egress::{GuardedClient, Lookup, Policy, Settings};
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
    build_state_with,
    config::GatewayConfig,
    decision::{DecisionRecord, DecisionSink},
};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::net::TcpListener;

const RESOURCE: &str = "https://auth.acme.test/mcp/crm";
const HOST: &str = "auth.acme.test";

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

#[derive(Clone, Default)]
struct Stub(Arc<HashMap<String, Vec<IpAddr>>>);

impl Stub {
    fn of(hosts: &[(&str, &str)]) -> Self {
        Self(Arc::new(
            hosts
                .iter()
                .map(|(host, ip)| (host.to_string(), vec![ip.parse().unwrap()]))
                .collect(),
        ))
    }
}

impl Lookup for Stub {
    async fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        self.0
            .get(host)
            .cloned()
            .ok_or_else(|| io::Error::other("unknown host"))
    }
}

#[derive(Default)]
struct AsState {
    forms: Mutex<Vec<HashMap<String, String>>>,
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
    singles: Mutex<Vec<Value>>,
    allowed: Mutex<Vec<String>>,
}

async fn evaluation(
    State(state): State<Arc<PdpState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some("Bearer pdp-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    state.singles.lock().unwrap().push(body.clone());
    let id = body["resource"]["id"].as_str().unwrap_or_default();
    let allowed = state.allowed.lock().unwrap().iter().any(|a| a == id);
    Json(json!({"decision": allowed, "context": {"reason_admin": {"policies": ["p"]}}}))
        .into_response()
}

#[derive(Default)]
struct CrmState {
    authorizations: Mutex<Vec<Option<String>>>,
}

async fn crm(
    State(state): State<Arc<CrmState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.authorizations.lock().unwrap().push(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    );
    Json(json!({
        "jsonrpc": "2.0",
        "id": body["id"],
        "result": {"content": [{"type": "text", "text": "invoices"}]},
    }))
    .into_response()
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
    crm: Arc<CrmState>,
    decisions: Collector,
    http: Client,
}

impl World {
    async fn start(upstream_url: impl Fn(u16) -> String, hosts: Stub, policy: Policy) -> Self {
        let crm_state = Arc::new(CrmState::default());
        let crm_port = serve(
            Router::new()
                .route("/mcp", post(crm))
                .with_state(Arc::clone(&crm_state)),
        )
        .await;
        Self::start_around(upstream_url(crm_port), hosts, policy, crm_state).await
    }

    async fn start_around(
        upstream_url: String,
        hosts: Stub,
        policy: Policy,
        crm_state: Arc<CrmState>,
    ) -> Self {
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
                .with_state(Arc::clone(&pdp)),
        )
        .await;

        let source = format!(
            r#"
            [[upstreams]]
            canonical_uri = "{RESOURCE}"
            issuer = "{issuer}"
            upstream_url = "{upstream_url}"
            server_name = "crm"
            egress = "public"

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
        let public = GuardedClient::with(
            hosts,
            policy,
            Settings {
                connect_timeout: Duration::from_secs(2),
                timeout: None,
            },
        )
        .unwrap();
        let decisions = Collector::default();
        let state = build_state_with(
            GatewayConfig::from_toml(&source).unwrap(),
            decisions.clone(),
            public,
        )
        .unwrap();
        let gateway_port = serve(router(state)).await;

        Self {
            gateway: format!("http://127.0.0.1:{gateway_port}"),
            issuer,
            auth,
            pdp,
            crm: crm_state,
            decisions,
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    fn token(&self) -> String {
        let claims = json!({
            "iss": self.issuer,
            "aud": RESOURCE,
            "sub": "alice",
            "azp": "claude",
            "scope": "mcp.tools",
            "exp": now() + 600,
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

    async fn call_tool(&self, token: &str, name: &str) -> reqwest::Response {
        self.http
            .post(format!("{}/mcp/crm", self.gateway))
            .header("host", HOST)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-method", "tools/call")
            .header("mcp-name", name)
            .bearer_auth(token)
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": name, "arguments": {}},
            }))
            .send()
            .await
            .unwrap()
    }
}

fn open() -> Policy {
    Policy::local_testing()
}

#[tokio::test]
async fn spec_ms_5_the_pdp_decides_on_the_servers_prefixed_tool_and_the_server_gets_an_exchanged_token()
 {
    let w = World::start(
        |port| format!("http://crm.example:{port}/mcp"),
        Stub::of(&[("crm.example", "127.0.0.1")]),
        open(),
    )
    .await;
    w.pdp
        .allowed
        .lock()
        .unwrap()
        .push("crm/list_invoices".to_string());
    let agent_token = w.token();

    let response = w.call_tool(&agent_token, "list_invoices").await;

    assert_eq!(response.status(), 200);
    let singles = w.pdp.singles.lock().unwrap().clone();
    assert_eq!(singles.len(), 1);
    assert_eq!(singles[0]["resource"]["id"], "crm/list_invoices");
    assert_eq!(singles[0]["resource"]["type"], "Tool");
    let received = w.crm.authorizations.lock().unwrap().clone();
    assert_eq!(received, vec![Some("Bearer delegated-1".to_string())]);
    assert!(!received[0].as_ref().unwrap().contains(&agent_token));
    let forms = w.auth.forms.lock().unwrap().clone();
    let exchange = forms
        .iter()
        .find(|form| form["grant_type"].contains("token-exchange"))
        .expect("an exchange");
    assert_eq!(exchange["audience"], "mcp-crm");
    assert_eq!(exchange["subject_token"], agent_token);
    assert_eq!(exchange["actor_token"], "actor-token");
    let records = w.decisions.0.lock().unwrap().clone();
    assert_eq!(records[0].server, "crm");
    assert_eq!(records[0].decision, "allow");
}

#[tokio::test]
async fn spec_ms_5_a_tool_missing_from_any_catalogue_is_still_denied_by_default() {
    let w = World::start(
        |port| format!("http://crm.example:{port}/mcp"),
        Stub::of(&[("crm.example", "127.0.0.1")]),
        open(),
    )
    .await;

    let response = w.call_tool(&w.token(), "never_listed").await;

    assert_eq!(response.status(), 403);
    assert!(w.crm.authorizations.lock().unwrap().is_empty());
    assert_eq!(w.auth.exchanges.load(Ordering::SeqCst), 0);
    assert_eq!(w.decisions.0.lock().unwrap()[0].decision, "deny");
}

#[tokio::test]
async fn spec_ms_4_a_host_resolving_to_loopback_is_never_connected_to() {
    let crm_state = Arc::new(CrmState::default());
    let port = serve(
        Router::new()
            .route("/mcp", post(crm))
            .with_state(Arc::clone(&crm_state)),
    )
    .await;
    let w = World::start_around(
        format!("https://evil.example:{port}/mcp"),
        Stub::of(&[("evil.example", "127.0.0.1")]),
        Policy::PUBLIC,
        crm_state,
    )
    .await;
    w.pdp
        .allowed
        .lock()
        .unwrap()
        .push("crm/list_invoices".to_string());

    let response = w.call_tool(&w.token(), "list_invoices").await;

    assert_eq!(response.status(), 502);
    assert!(w.crm.authorizations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spec_ms_4_the_cloud_metadata_address_is_never_connected_to() {
    let w = World::start(
        |_| "https://169.254.169.254/latest/meta-data".to_string(),
        Stub::default(),
        Policy::PUBLIC,
    )
    .await;
    w.pdp
        .allowed
        .lock()
        .unwrap()
        .push("crm/list_invoices".to_string());

    let response = w.call_tool(&w.token(), "list_invoices").await;

    assert_eq!(response.status(), 502);
}

#[tokio::test]
async fn spec_ms_4_a_redirect_from_a_customer_server_is_handed_back_not_followed() {
    let inside_hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&inside_hits);
    let inside_port = serve(Router::new().fallback(move || {
        let counted = Arc::clone(&counted);
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            "inside"
        }
    }))
    .await;
    let location = format!("http://127.0.0.1:{inside_port}/secret");
    let redirecting_port = serve(Router::new().route(
        "/mcp",
        post(move || {
            let location = location.clone();
            async move { (StatusCode::FOUND, [(header::LOCATION, location)]).into_response() }
        }),
    ))
    .await;
    let w = World::start_around(
        format!("http://hop.example:{redirecting_port}/mcp"),
        Stub::of(&[("hop.example", "127.0.0.1")]),
        open(),
        Arc::new(CrmState::default()),
    )
    .await;
    w.pdp
        .allowed
        .lock()
        .unwrap()
        .push("crm/list_invoices".to_string());

    let response = w.call_tool(&w.token(), "list_invoices").await;

    assert_eq!(response.status(), 302);
    assert_eq!(inside_hits.load(Ordering::SeqCst), 0);
}
