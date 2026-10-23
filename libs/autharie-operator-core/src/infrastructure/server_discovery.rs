use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::{AgentServerPhase, AgentServerTool};
use autharie_egress::{EgressError, GuardedClient, Settings, read_capped};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};

use crate::domain::agents::servers::{
    Discovered, DiscoveryTarget, MAX_TOOL_DESCRIPTION_CHARS, MAX_TOOLS, cut,
};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);
const DEFAULT_MAX_BODY: usize = 1024 * 1024;
const MAX_PAGES: usize = 20;
const PROTOCOL_VERSION: &str = "2025-06-18";
const GRANT_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const TOKEN_TYPE_ACCESS: &str = "urn:ietf:params:oauth:token-type:access_token";

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub deadline: Duration,
    pub max_body: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            max_body: DEFAULT_MAX_BODY,
        }
    }
}

#[derive(Debug)]
enum Failure {
    Refused(String),
    Unreachable(String),
    Invalid(String),
}

impl Failure {
    fn from_egress(error: &EgressError) -> Self {
        match error {
            EgressError::Blocked { .. } => {
                Self::Refused("the server's address is not a public one".to_string())
            }
            EgressError::Url => Self::Refused("the URL is not a public https URL".to_string()),
            EgressError::Unresolved { .. } => {
                Self::Unreachable("the server's name does not resolve".to_string())
            }
            EgressError::TooLarge { limit } => {
                Self::Unreachable(format!("the answer is larger than {limit} bytes"))
            }
            EgressError::Timeout => {
                Self::Unreachable("the server did not answer in time".to_string())
            }
            EgressError::Unreachable => {
                Self::Unreachable("the server could not be reached".to_string())
            }
        }
    }

    fn found(self) -> Discovered {
        match self {
            Self::Refused(message) => Discovered::failed(AgentServerPhase::Refused, message),
            Self::Unreachable(message) => {
                Discovered::failed(AgentServerPhase::Unreachable, message)
            }
            Self::Invalid(message) => Discovered::failed(AgentServerPhase::Invalid, message),
        }
    }
}

struct Session {
    id: Option<String>,
    version: String,
}

pub struct McpDiscoverer {
    tokens: Client,
    guarded: GuardedClient,
    limits: Limits,
}

impl McpDiscoverer {
    pub fn new() -> Result<Self, reqwest::Error> {
        let tokens = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DEFAULT_DEADLINE)
            .build()?;
        let guarded = GuardedClient::new(Settings {
            connect_timeout: Duration::from_secs(5),
            timeout: Some(DEFAULT_DEADLINE),
        })?;
        Ok(Self::with(tokens, guarded, Limits::default()))
    }

    pub fn with(tokens: Client, guarded: GuardedClient, limits: Limits) -> Self {
        Self {
            tokens,
            guarded,
            limits,
        }
    }

    pub async fn discover(&self, target: &DiscoveryTarget<'_>) -> Discovered {
        match tokio::time::timeout(self.limits.deadline, self.run(target)).await {
            Ok(Ok(found)) => found,
            Ok(Err(failure)) => failure.found(),
            Err(_) => Failure::Unreachable("the server did not answer in time".to_string()).found(),
        }
    }

    async fn run(&self, target: &DiscoveryTarget<'_>) -> Result<Discovered, Failure> {
        let token = self.token(target).await.map_err(Failure::Unreachable)?;

        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "autharie-operator", "version": "1" },
            },
        });
        let (answer, id) = self
            .post(target.url, &token, None, &initialize, Some(1))
            .await?;
        let session = Session {
            id,
            version: answer["result"]["protocolVersion"]
                .as_str()
                .unwrap_or(PROTOCOL_VERSION)
                .to_string(),
        };

        let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        self.post(target.url, &token, Some(&session), &initialized, None)
            .await?;

        let mut tools = Vec::new();
        let mut dropped = 0u32;
        let mut cursor: Option<String> = None;
        let mut more = false;
        for page in 0..MAX_PAGES {
            let mut request = json!({
                "jsonrpc": "2.0",
                "id": page + 2,
                "method": "tools/list",
            });
            if let Some(cursor) = &cursor {
                request["params"] = json!({ "cursor": cursor });
            }
            let (answer, _) = self
                .post(
                    target.url,
                    &token,
                    Some(&session),
                    &request,
                    Some(page as u64 + 2),
                )
                .await?;
            let listed = answer["result"]["tools"].as_array().ok_or_else(|| {
                Failure::Invalid("the answer to tools/list holds no tools".to_string())
            })?;
            for tool in listed {
                match read_tool(tool) {
                    Some(read) if tools.len() < MAX_TOOLS * 2 => tools.push(read),
                    _ => dropped = dropped.saturating_add(1),
                }
            }
            cursor = answer["result"]["nextCursor"]
                .as_str()
                .filter(|next| !next.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
            more = page + 1 == MAX_PAGES;
        }

        let mut found = Discovered::listed(tools, dropped);
        if more {
            found.message = Some(format!("the list goes on after {MAX_PAGES} pages"));
        }
        Ok(found)
    }

    async fn token(&self, target: &DiscoveryTarget<'_>) -> Result<String, String> {
        let own = self
            .token_request(
                target,
                &[
                    ("grant_type", "client_credentials"),
                    ("audience", target.audience),
                ],
            )
            .await?;
        if audience_of(&own).is_some_and(|audiences| audiences.iter().any(|a| a == target.audience))
        {
            return Ok(own);
        }

        self.token_request(
            target,
            &[
                ("grant_type", GRANT_EXCHANGE),
                ("subject_token", &own),
                ("subject_token_type", TOKEN_TYPE_ACCESS),
                ("actor_token", &own),
                ("actor_token_type", TOKEN_TYPE_ACCESS),
                ("audience", target.audience),
            ],
        )
        .await
    }

    async fn token_request(
        &self,
        target: &DiscoveryTarget<'_>,
        form: &[(&str, &str)],
    ) -> Result<String, String> {
        let mut fields: Vec<(&str, &str)> = form.to_vec();
        fields.push(("client_id", target.client_id));
        fields.push(("client_secret", target.client_secret));
        let response = self
            .tokens
            .post(target.token_url)
            .form(&fields)
            .send()
            .await
            .map_err(|_| "FerrisKey did not answer the token request".to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "FerrisKey refused a token for {} ({})",
                target.audience,
                response.status().as_u16()
            ));
        }
        let bytes = read_capped(response, self.limits.max_body)
            .await
            .map_err(|_| "FerrisKey's token answer could not be read".to_string())?;
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body["access_token"].as_str().map(str::to_string))
            .ok_or_else(|| "FerrisKey's token answer holds no token".to_string())
    }

    async fn post(
        &self,
        url: &str,
        token: &str,
        session: Option<&Session>,
        body: &Value,
        expect: Option<u64>,
    ) -> Result<(Value, Option<String>), Failure> {
        let payload = serde_json::to_vec(body)
            .map_err(|_| Failure::Invalid("the request could not be written".to_string()))?;
        let mut request = self
            .guarded
            .request(Method::POST, url)
            .map_err(|error| Failure::from_egress(&error))?
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .body(payload);
        if let Some(session) = session {
            request = request.header("mcp-protocol-version", session.version.as_str());
            if let Some(id) = &session.id {
                request = request.header("mcp-session-id", id.as_str());
            }
        }

        let response = request
            .send()
            .await
            .map_err(|error| Failure::from_egress(&EgressError::classify(&error)))?;
        let status = response.status();
        if status.is_redirection() {
            return Err(Failure::Refused(
                "the server answered with a redirect, which is not followed".to_string(),
            ));
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(Failure::Unreachable(format!(
                "the server refused the token ({})",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(Failure::Unreachable(format!(
                "the server answered {}",
                status.as_u16()
            )));
        }

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let streamed = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|kind| kind.to_ascii_lowercase().starts_with("text/event-stream"));
        let bytes = read_capped(response, self.limits.max_body)
            .await
            .map_err(|error| Failure::from_egress(&error))?;

        let Some(expected) = expect else {
            return Ok((Value::Null, session_id));
        };
        let message = if streamed {
            message_in_stream(&bytes, expected)
        } else {
            serde_json::from_slice::<Value>(&bytes).ok()
        }
        .ok_or_else(|| Failure::Invalid("the answer is not a JSON-RPC message".to_string()))?;

        if let Some(error) = message.get("error") {
            let reason = error["message"].as_str().unwrap_or("no reason given");
            return Err(Failure::Invalid(format!(
                "the server answered an error: {}",
                cut(reason, 200)
            )));
        }
        if message.get("result").is_none() {
            return Err(Failure::Invalid(
                "the answer is not a JSON-RPC result".to_string(),
            ));
        }
        Ok((message, session_id))
    }
}

fn read_tool(tool: &Value) -> Option<AgentServerTool> {
    let name = tool["name"].as_str()?;
    Some(AgentServerTool {
        name: name.to_string(),
        description: tool["description"]
            .as_str()
            .map(|text| cut(text, MAX_TOOL_DESCRIPTION_CHARS)),
        read_only: tool["annotations"]["readOnlyHint"]
            .as_bool()
            .unwrap_or(false),
        destructive: tool["annotations"]["destructiveHint"]
            .as_bool()
            .unwrap_or(false),
    })
}

fn message_in_stream(bytes: &[u8], expected: u64) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?.replace("\r\n", "\n");
    text.split("\n\n")
        .filter_map(|event| {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
                .collect();
            serde_json::from_str::<Value>(&data.join("\n")).ok()
        })
        .find(|message| message["id"].as_u64() == Some(expected))
}

fn audience_of(token: &str) -> Option<Vec<String>> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    match &claims["aud"] {
        Value::String(one) => Some(vec![one.clone()]),
        Value::Array(many) => Some(
            many.iter()
                .filter_map(|aud| aud.as_str().map(str::to_string))
                .collect(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use autharie_egress::{Lookup, Policy};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::{Form, Json, Router};
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct Hosts(Arc<HashMap<String, Vec<IpAddr>>>);

    impl Hosts {
        fn of(entries: &[(&str, &str)]) -> Self {
            Self(Arc::new(
                entries
                    .iter()
                    .map(|(host, ip)| (host.to_string(), vec![ip.parse().unwrap()]))
                    .collect(),
            ))
        }
    }

    impl Lookup for Hosts {
        async fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
            self.0
                .get(host)
                .cloned()
                .ok_or_else(|| io::Error::other("unknown host"))
        }
    }

    async fn serve(app: Router) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        port
    }

    fn jwt(audience: Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(json!({ "aud": audience }).to_string());
        format!("e30.{payload}.sig")
    }

    #[derive(Default)]
    struct Tokens {
        forms: Mutex<Vec<HashMap<String, String>>>,
        own_audience: Mutex<Option<String>>,
        refuse: Mutex<bool>,
    }

    async fn token_endpoint(
        State(state): State<Arc<Tokens>>,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        state.forms.lock().unwrap().push(form.clone());
        if *state.refuse.lock().unwrap() {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let token = match form["grant_type"].as_str() {
            "client_credentials" => match state.own_audience.lock().unwrap().clone() {
                Some(audience) => jwt(json!(audience)),
                None => jwt(json!("account")),
            },
            _ => jwt(json!(["mcp-crm", "account"])),
        };
        Json(json!({ "access_token": token, "expires_in": 300 })).into_response()
    }

    #[derive(Clone)]
    enum Mode {
        Tools(Vec<Value>),
        Paged,
        Stream(Vec<Value>),
        Huge,
        Slow,
        Fail(StatusCode),
        RpcError,
        Garbage,
        Redirect(String),
    }

    struct Server {
        mode: Mode,
        authorizations: Mutex<Vec<String>>,
        methods: Mutex<Vec<String>>,
        sessions: Mutex<Vec<Option<String>>>,
        hits: AtomicUsize,
    }

    fn tools(count: usize) -> Vec<Value> {
        (0..count)
            .map(|i| json!({ "name": format!("tool_{i}"), "description": "d" }))
            .collect()
    }

    async fn mcp(
        State(state): State<Arc<Server>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        state.hits.fetch_add(1, Ordering::SeqCst);
        state.authorizations.lock().unwrap().push(
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        );
        state.sessions.lock().unwrap().push(
            headers
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
        );
        let method = body["method"].as_str().unwrap_or_default().to_string();
        state.methods.lock().unwrap().push(method.clone());
        let id = body["id"].clone();

        match &state.mode {
            Mode::Fail(status) => return status.into_response(),
            Mode::Redirect(to) => {
                return (StatusCode::FOUND, [("location", to.clone())]).into_response();
            }
            Mode::Slow => {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Mode::Garbage => return "not json at all".into_response(),
            Mode::RpcError => {
                return Json(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": "no such method" }
                }))
                .into_response();
            }
            Mode::Huge => {
                return Json(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "tools": [], "padding": "a".repeat(2 * 1024 * 1024) }
                }))
                .into_response();
            }
            _ => {}
        }

        match method.as_str() {
            "initialize" => (
                [("mcp-session-id", "session-1")],
                Json(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "protocolVersion": "2025-03-26", "capabilities": {} }
                })),
            )
                .into_response(),
            "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
            "tools/list" => {
                let cursor = body["params"]["cursor"].as_str();
                let result = match (&state.mode, cursor) {
                    (Mode::Tools(list), _) | (Mode::Stream(list), _) => {
                        json!({ "tools": list })
                    }
                    (Mode::Paged, None) => json!({ "tools": tools(2), "nextCursor": "page-2" }),
                    (Mode::Paged, Some(_)) => json!({
                        "tools": [{ "name": "last_one" }]
                    }),
                    _ => json!({ "tools": [] }),
                };
                let message = json!({ "jsonrpc": "2.0", "id": id, "result": result });
                if matches!(state.mode, Mode::Stream(_)) {
                    (
                        [("content-type", "text/event-stream")],
                        format!("event: message\ndata: {message}\n\n"),
                    )
                        .into_response()
                } else {
                    Json(message).into_response()
                }
            }
            _ => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    struct World {
        discoverer: McpDiscoverer,
        tokens: Arc<Tokens>,
        server: Arc<Server>,
        url: String,
        token_url: String,
    }

    async fn world(mode: Mode, policy: Policy, host: &str, address: &str) -> World {
        let tokens = Arc::new(Tokens::default());
        let token_port = serve(
            Router::new()
                .route("/token", post(token_endpoint))
                .with_state(Arc::clone(&tokens)),
        )
        .await;
        let server = Arc::new(Server {
            mode,
            authorizations: Mutex::default(),
            methods: Mutex::default(),
            sessions: Mutex::default(),
            hits: AtomicUsize::new(0),
        });
        let port = serve(
            Router::new()
                .route("/mcp", post(mcp))
                .with_state(Arc::clone(&server)),
        )
        .await;
        let limits = Limits {
            deadline: Duration::from_millis(1500),
            max_body: 1024 * 1024,
        };
        let guarded = GuardedClient::with(
            Hosts::of(&[(host, address)]),
            policy,
            Settings {
                connect_timeout: Duration::from_secs(2),
                timeout: Some(limits.deadline),
            },
        )
        .unwrap();
        World {
            discoverer: McpDiscoverer::with(Client::new(), guarded, limits),
            tokens,
            server,
            url: format!("http://{host}:{port}/mcp"),
            token_url: format!("http://127.0.0.1:{token_port}/token"),
        }
    }

    async fn open(mode: Mode) -> World {
        world(mode, Policy::local_testing(), "crm.example", "127.0.0.1").await
    }

    async fn run(world: &World) -> Discovered {
        world
            .discoverer
            .discover(&DiscoveryTarget {
                token_url: &world.token_url,
                client_id: "mcp-gateway",
                client_secret: "gw-secret",
                audience: "mcp-crm",
                url: &world.url,
            })
            .await
    }

    #[tokio::test]
    async fn spec_ms_3_the_tools_come_back_with_their_hints() {
        let w = open(Mode::Tools(vec![
            json!({ "name": "list_invoices", "description": "lists",
                    "annotations": { "readOnlyHint": true } }),
            json!({ "name": "delete_invoice",
                    "annotations": { "readOnlyHint": false, "destructiveHint": true } }),
            json!({ "name": "plain" }),
        ]))
        .await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Ready);
        assert_eq!(found.dropped, 0);
        let list = &found.tools[0];
        assert_eq!(list.name, "list_invoices");
        assert_eq!(list.description.as_deref(), Some("lists"));
        assert!(list.read_only && !list.destructive);
        let delete = &found.tools[1];
        assert!(!delete.read_only && delete.destructive);
        let plain = &found.tools[2];
        assert!(!plain.read_only && !plain.destructive);
        assert_eq!(
            *w.server.methods.lock().unwrap(),
            ["initialize", "notifications/initialized", "tools/list"]
        );
    }

    #[tokio::test]
    async fn the_server_gets_the_token_whose_audience_is_its_own_and_the_session_it_opened() {
        let w = open(Mode::Tools(tools(1))).await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        run(&w).await;

        let forms = w.tokens.forms.lock().unwrap().clone();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0]["grant_type"], "client_credentials");
        assert_eq!(forms[0]["audience"], "mcp-crm");
        assert_eq!(forms[0]["client_id"], "mcp-gateway");
        let received = w.server.authorizations.lock().unwrap().clone();
        assert!(
            received
                .iter()
                .all(|value| *value == format!("Bearer {}", jwt(json!("mcp-crm"))))
        );
        let sessions = w.server.sessions.lock().unwrap().clone();
        assert_eq!(sessions[0], None);
        assert_eq!(sessions[1], Some("session-1".to_string()));
        assert_eq!(sessions[2], Some("session-1".to_string()));
    }

    #[tokio::test]
    async fn a_token_for_the_wrong_audience_is_exchanged_for_the_right_one() {
        let w = open(Mode::Tools(tools(1))).await;

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Ready);
        let forms = w.tokens.forms.lock().unwrap().clone();
        assert_eq!(forms.len(), 2);
        assert!(forms[1]["grant_type"].contains("token-exchange"));
        assert_eq!(forms[1]["audience"], "mcp-crm");
        assert_eq!(forms[1]["subject_token"], jwt(json!("account")));
        assert_eq!(forms[1]["actor_token"], jwt(json!("account")));
        let received = w.server.authorizations.lock().unwrap().clone();
        assert!(received[0].contains(&jwt(json!(["mcp-crm", "account"]))));
    }

    #[tokio::test]
    async fn a_token_that_cannot_be_had_is_unreachable_and_never_reaches_the_server() {
        let w = open(Mode::Tools(tools(1))).await;
        *w.tokens.refuse.lock().unwrap() = true;

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Unreachable);
        assert!(!found.message.unwrap().contains("gw-secret"));
        assert_eq!(w.server.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_event_stream_answer_is_read_like_a_json_one() {
        let w = open(Mode::Stream(tools(2))).await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Ready);
        assert_eq!(found.tools.len(), 2);
    }

    #[tokio::test]
    async fn a_list_that_continues_is_followed_to_its_end() {
        let w = open(Mode::Paged).await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        let names: Vec<_> = found.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["tool_0", "tool_1", "last_one"]);
    }

    #[tokio::test]
    async fn spec_ms_8_five_hundred_tools_are_cut_to_two_hundred_and_the_rest_counted() {
        let w = open(Mode::Tools(tools(500))).await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Ready);
        assert_eq!(found.tools.len(), 200);
        assert_eq!(found.dropped, 300);
    }

    #[tokio::test]
    async fn names_outside_the_alphabet_are_dropped_and_counted() {
        let w = open(Mode::Tools(vec![
            json!({ "name": "good" }),
            json!({ "name": "bad name" }),
            json!({ "name": "../etc" }),
            json!({ "description": "no name" }),
        ]))
        .await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        assert_eq!(found.tools.len(), 1);
        assert_eq!(found.dropped, 3);
    }

    #[tokio::test]
    async fn spec_ms_8_a_two_megabyte_answer_is_unreachable_not_a_crash() {
        let w = open(Mode::Huge).await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Unreachable);
        assert!(found.tools.is_empty());
    }

    #[tokio::test]
    async fn spec_ms_8_a_server_that_takes_thirty_seconds_is_unreachable_in_time() {
        let w = open(Mode::Slow).await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());
        let started = std::time::Instant::now();

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Unreachable);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn spec_ms_4_a_host_resolving_to_loopback_is_refused_and_never_connected_to() {
        let w = world(
            Mode::Tools(tools(1)),
            Policy::PUBLIC,
            "evil.example",
            "127.0.0.1",
        )
        .await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());
        let url = w.url.replace("http://", "https://");

        let found = w
            .discoverer
            .discover(&DiscoveryTarget {
                token_url: &w.token_url,
                client_id: "mcp-gateway",
                client_secret: "gw-secret",
                audience: "mcp-crm",
                url: &url,
            })
            .await;

        assert_eq!(found.phase, AgentServerPhase::Refused);
        assert_eq!(w.server.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn spec_ms_4_the_cloud_metadata_address_is_refused() {
        let w = world(
            Mode::Tools(tools(1)),
            Policy::PUBLIC,
            "meta.example",
            "169.254.169.254",
        )
        .await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = w
            .discoverer
            .discover(&DiscoveryTarget {
                token_url: &w.token_url,
                client_id: "mcp-gateway",
                client_secret: "gw-secret",
                audience: "mcp-crm",
                url: "https://meta.example/latest/meta-data",
            })
            .await;

        assert_eq!(found.phase, AgentServerPhase::Refused);
    }

    #[tokio::test]
    async fn spec_ms_4_a_redirect_to_another_host_is_not_followed_and_the_status_says_refused() {
        let inside = Arc::new(Server {
            mode: Mode::Tools(tools(1)),
            authorizations: Mutex::default(),
            methods: Mutex::default(),
            sessions: Mutex::default(),
            hits: AtomicUsize::new(0),
        });
        let inside_port = serve(
            Router::new()
                .route("/mcp", post(mcp))
                .with_state(Arc::clone(&inside)),
        )
        .await;
        let w = open(Mode::Redirect(format!(
            "http://127.0.0.1:{inside_port}/mcp"
        )))
        .await;
        *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

        let found = run(&w).await;

        assert_eq!(found.phase, AgentServerPhase::Refused);
        assert_eq!(inside.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_server_that_answers_an_error_status_is_unreachable_and_names_it() {
        for (status, wanted) in [
            (StatusCode::INTERNAL_SERVER_ERROR, "500"),
            (StatusCode::UNAUTHORIZED, "401"),
            (StatusCode::FORBIDDEN, "403"),
        ] {
            let w = open(Mode::Fail(status)).await;
            *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

            let found = run(&w).await;

            assert_eq!(found.phase, AgentServerPhase::Unreachable, "{status}");
            assert!(found.message.unwrap().contains(wanted));
        }
    }

    #[tokio::test]
    async fn a_server_that_does_not_speak_json_rpc_is_invalid() {
        for mode in [Mode::Garbage, Mode::RpcError] {
            let w = open(mode).await;
            *w.tokens.own_audience.lock().unwrap() = Some("mcp-crm".to_string());

            let found = run(&w).await;

            assert_eq!(found.phase, AgentServerPhase::Invalid);
            assert!(found.tools.is_empty());
        }
    }

    #[test]
    fn the_audience_of_a_token_is_read_whether_it_is_one_or_many() {
        assert_eq!(audience_of(&jwt(json!("a"))), Some(vec!["a".to_string()]));
        assert_eq!(
            audience_of(&jwt(json!(["a", "b"]))),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(audience_of("garbage"), None);
        assert_eq!(audience_of("a.%%%.c"), None);
    }
}
