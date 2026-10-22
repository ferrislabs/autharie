use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{SecondsFormat, Utc};
use reqwest::Client;
use serde_json::{Value, json};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    authzen::{self, Decision, PdpError},
    decision::{DecisionRecord, DecisionSink, KIND},
    rpc::{self, Operation, Rejection},
    upstream::Routes,
    upstream::Upstream,
    verifier::{Principal, VerifyError},
};

const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_ID: &str = "x-request-id";

const FORWARDED_REQUEST_HEADERS: [&str; 5] = [
    "content-type",
    "accept",
    "last-event-id",
    "mcp-session-id",
    "mcp-protocol-version",
];

const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub struct Gateway<S> {
    pub routes: Routes,
    pub sink: S,
    pub http: Client,
}

pub struct AppState<S>(Arc<Gateway<S>>);

impl<S> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<S> AppState<S> {
    pub fn new(gateway: Gateway<S>) -> Self {
        Self(Arc::new(gateway))
    }
}

pub fn router<S>(state: AppState<S>) -> Router
where
    S: DecisionSink + Send + Sync + 'static,
{
    Router::new()
        .route("/healthz", get(health))
        .fallback(handle::<S>)
        .with_state(state)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

fn generic(status: StatusCode, message: &'static str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn rejection_response(rejection: &Rejection) -> Response {
    let status = StatusCode::from_u16(rejection.http_status).unwrap_or(StatusCode::FORBIDDEN);
    (status, Json(rejection.body())).into_response()
}

fn unauthorized(upstream: &Upstream, invalid_token: bool) -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    if let Ok(value) = HeaderValue::from_str(&upstream.challenge(invalid_token)) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

fn request_host(parts: &Parts) -> &str {
    parts
        .headers
        .get("x-forwarded-host")
        .or_else(|| parts.headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .or_else(|| parts.uri.authority().map(|a| a.as_str()))
        .unwrap_or_default()
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get(REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128 && v.is_ascii())
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned)
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

struct Call<'a, S> {
    gateway: &'a Gateway<S>,
    upstream: &'a Upstream,
    principal: &'a Principal,
    request_id: &'a str,
}

impl<S: DecisionSink> Call<'_, S> {
    fn record(
        &self,
        method: &str,
        tool: Option<&str>,
        allowed: bool,
        policies: Vec<String>,
        reason: Option<&'static str>,
    ) {
        let (subject_type, subject_id) = authzen::subject_kind(self.principal);
        self.gateway.sink.record(&DecisionRecord {
            kind: KIND,
            time: now(),
            server: self.upstream.server_name.clone(),
            method: method.to_owned(),
            tool: tool.map(str::to_owned),
            subject_type,
            subject_id: subject_id.to_owned(),
            agent: self.principal.client_id.clone(),
            delegated: self.principal.delegated,
            decision: if allowed { "allow" } else { "deny" },
            policies,
            reason,
            request_id: self.request_id.to_owned(),
        });
    }

    fn record_outcome(
        &self,
        method: &str,
        tool: Option<&str>,
        outcome: &Result<Decision, PdpError>,
    ) {
        match outcome {
            Ok(decision) => self.record(
                method,
                tool,
                decision.allowed,
                decision.policies.clone(),
                (!decision.allowed).then_some("no_permitting_policy"),
            ),
            Err(error) => self.record(method, tool, false, Vec::new(), Some(error.reason())),
        }
    }

    async fn gate(&self, id: &Value, method: &str, target: &rpc::Target) -> Result<(), Rejection> {
        let annotations = match target.operation {
            Operation::CallTool => self.upstream.annotations_of(&target.name),
            _ => json!({}),
        };
        let outcome = self
            .upstream
            .pdp
            .evaluate(
                &authzen::subject(self.principal),
                target.operation.action(),
                &authzen::resource(
                    target.operation,
                    &self.upstream.server_name,
                    &target.name,
                    &annotations,
                ),
                &authzen::context(self.principal, &target.arguments, Utc::now()),
            )
            .await;
        let tool = matches!(target.operation, Operation::CallTool).then_some(target.name.as_str());
        self.record_outcome(method, tool, &outcome);
        match outcome {
            Ok(Decision { allowed: true, .. }) => Ok(()),
            _ => Err(Rejection::denied(id.clone())),
        }
    }

    async fn filter_tools(&self, message: &mut Value) -> Result<(), PdpError> {
        let Some(tools) = message
            .get_mut("result")
            .and_then(|r| r.get_mut("tools"))
            .and_then(Value::as_array_mut)
        else {
            return Ok(());
        };
        tools.retain(|tool| tool.get("name").and_then(Value::as_str).is_some());
        let names: Vec<String> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let empty = json!({});
        let resources: Vec<Value> = tools
            .iter()
            .zip(&names)
            .map(|(tool, name)| {
                let annotations = tool.get("annotations").unwrap_or(&empty);
                self.upstream.remember_annotations(name, annotations);
                authzen::resource(
                    Operation::CallTool,
                    &self.upstream.server_name,
                    name,
                    annotations,
                )
            })
            .collect();
        let decisions = self
            .upstream
            .pdp
            .evaluate_many(
                &authzen::subject(self.principal),
                Operation::CallTool.action(),
                &resources,
                &authzen::context(self.principal, &serde_json::Map::new(), Utc::now()),
            )
            .await;
        match decisions {
            Ok(decisions) => {
                for (name, decision) in names.iter().zip(&decisions) {
                    self.record_outcome("tools/list", Some(name), &Ok(decision.clone()));
                }
                let mut allowed = decisions.iter().map(|d| d.allowed);
                tools.retain(|_| allowed.next().unwrap_or(false));
                Ok(())
            }
            Err(error) => {
                self.record_outcome("tools/list", None, &Err(error));
                Err(error)
            }
        }
    }

    async fn filter_sse(&self, text: &str) -> Result<String, PdpError> {
        let normalised = text.replace("\r\n", "\n");
        let mut out = Vec::new();
        for block in normalised.split_terminator("\n\n") {
            let mut lines: Vec<String> = block.lines().map(str::to_owned).collect();
            let data_positions: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.starts_with("data:"))
                .map(|(i, _)| i)
                .collect();
            if let Some(&first) = data_positions.first() {
                let joined = data_positions
                    .iter()
                    .map(|&i| {
                        let rest = &lines[i]["data:".len()..];
                        rest.strip_prefix(' ').unwrap_or(rest).to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Ok(mut message) = serde_json::from_str::<Value>(&joined) {
                    self.filter_tools(&mut message).await?;
                    let rewritten = format!("data: {message}");
                    for &i in data_positions.iter().skip(1).rev() {
                        lines.remove(i);
                    }
                    lines[first] = rewritten;
                }
            }
            out.push(lines.join("\n"));
        }
        Ok(format!("{}\n\n", out.join("\n\n")))
    }

    async fn filtered_list(&self, id: &Value, upstream: reqwest::Response) -> Response {
        let status = upstream.status();
        let headers = upstream.headers().clone();
        let Ok(bytes) = upstream.bytes().await else {
            return generic(StatusCode::BAD_GATEWAY, "upstream unavailable");
        };
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let rewritten: Result<Bytes, PdpError> = if !status.is_success() {
            Ok(bytes)
        } else if content_type.starts_with("application/json") {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(mut message) => self
                    .filter_tools(&mut message)
                    .await
                    .map(|()| Bytes::from(message.to_string())),
                Err(_) => return generic(StatusCode::BAD_GATEWAY, "upstream unavailable"),
            }
        } else if content_type.starts_with("text/event-stream") {
            match std::str::from_utf8(&bytes) {
                Ok(text) => self.filter_sse(text).await.map(Bytes::from),
                Err(_) => return generic(StatusCode::BAD_GATEWAY, "upstream unavailable"),
            }
        } else {
            return generic(StatusCode::BAD_GATEWAY, "upstream unavailable");
        };
        let body = match rewritten {
            Ok(body) => body,
            Err(_) => {
                let mut response = rejection_response(&Rejection::denied(id.clone()));
                no_store(response.headers_mut());
                return response;
            }
        };
        let mut response = Response::builder().status(status);
        for (name, value) in &headers {
            let name_str = name.as_str();
            if !HOP_BY_HOP.contains(&name_str)
                && name_str != "content-length"
                && name_str != "content-encoding"
            {
                response = response.header(name, value);
            }
        }
        let Ok(mut response) = response.body(Body::from(body)) else {
            return generic(StatusCode::BAD_GATEWAY, "upstream unavailable");
        };
        no_store(response.headers_mut());
        response
    }

    async fn forward(
        &self,
        parts: &Parts,
        body: Bytes,
        delegated: &str,
    ) -> Result<reqwest::Response, ()> {
        let mut outgoing = reqwest::header::HeaderMap::new();
        for (name, value) in &parts.headers {
            let name_str = name.as_str();
            if FORWARDED_REQUEST_HEADERS.contains(&name_str) || name_str.starts_with("mcp-") {
                outgoing.append(name.clone(), value.clone());
            }
        }
        let mut token = HeaderValue::from_str(&format!("Bearer {delegated}")).map_err(|_| ())?;
        token.set_sensitive(true);
        outgoing.insert(header::AUTHORIZATION, token);
        outgoing.insert(
            REQUEST_ID,
            HeaderValue::from_str(self.request_id).map_err(|_| ())?,
        );
        self.gateway
            .http
            .request(parts.method.clone(), &self.upstream.upstream_url)
            .headers(outgoing)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                warn!(%error, "upstream call failed");
            })
    }
}

fn no_store(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::VARY, HeaderValue::from_static("authorization"));
}

fn stream_back(upstream: reqwest::Response) -> Response {
    let mut response = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers() {
        if !HOP_BY_HOP.contains(&name.as_str()) {
            response = response.header(name, value);
        }
    }
    response
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| generic(StatusCode::BAD_GATEWAY, "upstream unavailable"))
}

async fn handle<S>(State(state): State<AppState<S>>, request: Request) -> Response
where
    S: DecisionSink + Send + Sync + 'static,
{
    let gateway = &*state.0;
    let (parts, body) = request.into_parts();
    let host = request_host(&parts);
    let path = parts.uri.path();

    if parts.method == Method::GET
        && let Some(upstream) = gateway.routes.for_metadata(host, path)
    {
        return Json(upstream.metadata()).into_response();
    }
    let Some(upstream) = gateway.routes.for_resource(host, path) else {
        return generic(StatusCode::NOT_FOUND, "not found");
    };

    let Some(token) = bearer(&parts.headers) else {
        return unauthorized(upstream, false);
    };
    let principal = match upstream.verifier.verify(token).await {
        Ok(principal) => principal,
        Err(VerifyError::Invalid) => return unauthorized(upstream, true),
        Err(VerifyError::Unavailable) => {
            return generic(StatusCode::SERVICE_UNAVAILABLE, "service unavailable");
        }
    };

    let Ok(body) = to_bytes(body, MAX_BODY_BYTES).await else {
        return generic(StatusCode::PAYLOAD_TOO_LARGE, "request too large");
    };

    let request_id = request_id(&parts.headers);
    let call = Call {
        gateway,
        upstream,
        principal: &principal,
        request_id: &request_id,
    };

    let message = if parts.method == Method::POST {
        let message = match rpc::parse(&body) {
            Ok(message) => message,
            Err(rejection) => {
                info!(sub = %principal.sub, agent = %principal.client_id, code = rejection.code, "request rejected");
                return rejection_response(&rejection);
            }
        };
        if let Err(rejection) = rpc::check_headers(&parts.headers, &message) {
            let tool = message
                .target
                .as_ref()
                .filter(|t| t.operation == Operation::CallTool)
                .map(|t| t.name.as_str());
            call.record(
                &message.method,
                tool,
                false,
                Vec::new(),
                Some("header_mismatch"),
            );
            return rejection_response(&rejection);
        }
        if let Some(target) = &message.target {
            let gated = call.gate(&message.id, &message.method, target).await;
            if let Err(rejection) = gated {
                return rejection_response(&rejection);
            }
        }
        Some(message)
    } else {
        None
    };

    let delegated = match upstream.exchanger.exchange(&principal, token).await {
        Ok(delegated) => delegated,
        Err(_) => return generic(StatusCode::BAD_GATEWAY, "upstream unavailable"),
    };

    let Ok(upstream_response) = call.forward(&parts, body, &delegated.access_token).await else {
        return generic(StatusCode::BAD_GATEWAY, "upstream unavailable");
    };
    match message {
        Some(message) if message.is_tools_list() => {
            call.filtered_list(&message.id, upstream_response).await
        }
        _ => stream_back(upstream_response),
    }
}
