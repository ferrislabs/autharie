use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use futures_util::future::join_all;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::warn;

use crate::{config::PdpConfig, rpc::Operation, verifier::Principal};

const SAFETY_MARGIN: Duration = Duration::from_secs(10);
const DEFAULT_LIFETIME: Duration = Duration::from_secs(60);
const FALLBACK_CONCURRENCY: usize = 16;

pub fn subject(principal: &Principal) -> Value {
    if principal.delegated {
        json!({
            "type": "User",
            "id": principal.sub,
            "properties": {
                "roles": principal.roles,
                "groups": principal.groups,
                "organisation": principal.organisation,
            },
        })
    } else {
        json!({ "type": "Client", "id": principal.client_id })
    }
}

pub fn subject_kind(principal: &Principal) -> (&'static str, &str) {
    if principal.delegated {
        ("User", &principal.sub)
    } else {
        ("Client", &principal.client_id)
    }
}

pub fn resource(operation: Operation, server: &str, name: &str, annotations: &Value) -> Value {
    match operation {
        Operation::CallTool => json!({
            "type": "Tool",
            "id": format!("{server}/{name}"),
            "properties": { "server": server, "tool": name, "annotations": annotations },
        }),
        Operation::ReadResource => json!({
            "type": "Resource",
            "id": format!("{server}/{name}"),
            "properties": { "server": server, "uri": name },
        }),
        Operation::GetPrompt => json!({
            "type": "Prompt",
            "id": format!("{server}/{name}"),
            "properties": { "server": server, "prompt": name },
        }),
    }
}

pub fn context(principal: &Principal, arguments: &Map<String, Value>, time: &str) -> Value {
    let primitives: Map<String, Value> = arguments
        .iter()
        .filter(|(_, v)| matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    json!({
        "agent": principal.client_id,
        "delegated": principal.delegated,
        "act": principal.act,
        "arguments": primitives,
        "time": time,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub policies: Vec<String>,
}

impl Decision {
    pub fn deny() -> Self {
        Self {
            allowed: false,
            policies: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PdpError {
    #[error("policy decision point timed out")]
    Timeout,
    #[error("policy decision point unavailable")]
    Unavailable,
    #[error("policy decision point has no such endpoint")]
    NotFound,
    #[error("policy decision point answered unreadably")]
    Malformed,
}

impl PdpError {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Timeout => "pdp_timeout",
            Self::Unavailable | Self::NotFound => "pdp_unavailable",
            Self::Malformed => "pdp_malformed",
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireDecision {
    decision: bool,
    #[serde(default)]
    context: Option<WireContext>,
}

#[derive(Debug, Deserialize)]
struct WireContext {
    #[serde(default)]
    reason_admin: Option<WireReason>,
}

#[derive(Debug, Deserialize)]
struct WireReason {
    #[serde(default)]
    policies: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WireBatch {
    evaluations: Vec<WireDecision>,
}

impl From<WireDecision> for Decision {
    fn from(wire: WireDecision) -> Self {
        Self {
            allowed: wire.decision,
            policies: wire
                .context
                .and_then(|c| c.reason_admin)
                .map(|r| r.policies)
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

pub struct AuthzenPdp {
    http: Client,
    config: PdpConfig,
    token_url: String,
    timeout: Duration,
    token: Mutex<Option<(String, Instant)>>,
}

impl AuthzenPdp {
    pub fn new(http: Client, config: PdpConfig, token_url: String, timeout: Duration) -> Self {
        Self {
            http,
            config,
            token_url,
            timeout,
            token: Mutex::new(None),
        }
    }

    pub async fn evaluate(
        &self,
        subject: &Value,
        action: &str,
        resource: &Value,
        context: &Value,
    ) -> Result<Decision, PdpError> {
        let body = json!({
            "subject": subject,
            "action": { "name": action },
            "resource": resource,
            "context": context,
        });
        let wire: WireDecision = self.post(&self.config.evaluation_url, &body).await?;
        Ok(wire.into())
    }

    pub async fn evaluate_many(
        &self,
        subject: &Value,
        action: &str,
        resources: &[Value],
        context: &Value,
    ) -> Result<Vec<Decision>, PdpError> {
        if resources.is_empty() {
            return Ok(Vec::new());
        }
        let body = json!({
            "subject": subject,
            "action": { "name": action },
            "context": context,
            "evaluations": resources
                .iter()
                .map(|resource| json!({ "resource": resource }))
                .collect::<Vec<_>>(),
        });
        match self
            .post::<WireBatch>(&self.config.evaluations_url, &body)
            .await
        {
            Ok(batch) if batch.evaluations.len() == resources.len() => {
                Ok(batch.evaluations.into_iter().map(Decision::from).collect())
            }
            Ok(_) => Err(PdpError::Malformed),
            Err(PdpError::NotFound) => {
                let mut decisions = Vec::with_capacity(resources.len());
                for chunk in resources.chunks(FALLBACK_CONCURRENCY) {
                    let pending: Vec<_> = chunk
                        .iter()
                        .map(|resource| self.evaluate(subject, action, resource, context))
                        .collect();
                    for outcome in join_all(pending).await {
                        decisions.push(outcome?);
                    }
                }
                Ok(decisions)
            }
            Err(error) => Err(error),
        }
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &Value,
    ) -> Result<T, PdpError> {
        let mut renewed = false;
        loop {
            let token = self.service_token().await?;
            let response = self
                .http
                .post(url)
                .timeout(self.timeout)
                .bearer_auth(&token)
                .json(body)
                .send()
                .await
                .map_err(classify)?;
            match response.status() {
                StatusCode::UNAUTHORIZED if !renewed => {
                    renewed = true;
                    *self.token.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                StatusCode::NOT_FOUND => return Err(PdpError::NotFound),
                status if status.is_success() => {
                    return response.json().await.map_err(|error| {
                        warn!(%error, "pdp answer unreadable");
                        PdpError::Malformed
                    });
                }
                status => {
                    warn!(%status, "pdp refused the evaluation");
                    return Err(PdpError::Unavailable);
                }
            }
        }
    }

    async fn service_token(&self) -> Result<String, PdpError> {
        let now = Instant::now();
        if let Some((token, until)) = self
            .token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            && now < *until
        {
            return Ok(token.clone());
        }
        let response = self
            .http
            .post(&self.token_url)
            .timeout(self.timeout)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.expose()),
            ])
            .send()
            .await
            .map_err(classify)?;
        if !response.status().is_success() {
            warn!(status = %response.status(), "pdp service token refused");
            return Err(PdpError::Unavailable);
        }
        let token: TokenResponse = response.json().await.map_err(|error| {
            warn!(%error, "pdp service token unreadable");
            PdpError::Malformed
        })?;
        let lifetime = token
            .expires_in
            .map_or(DEFAULT_LIFETIME, Duration::from_secs)
            .saturating_sub(SAFETY_MARGIN);
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((token.access_token.clone(), now + lifetime));
        Ok(token.access_token)
    }
}

fn classify(error: reqwest::Error) -> PdpError {
    warn!(%error, "pdp call failed");
    if error.is_timeout() {
        PdpError::Timeout
    } else {
        PdpError::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(delegated: bool) -> Principal {
        Principal {
            sub: "alice".into(),
            client_id: "claude".into(),
            scopes: vec![],
            exp: 0,
            roles: vec!["admin".into()],
            groups: vec!["support".into()],
            organisation: Some("acme".into()),
            act: Some(json!({"sub": "gw"})),
            delegated,
        }
    }

    #[test]
    fn a_delegated_agent_is_a_user_with_roles_groups_and_organisation() {
        assert_eq!(
            subject(&principal(true)),
            json!({
                "type": "User",
                "id": "alice",
                "properties": {"roles": ["admin"], "groups": ["support"], "organisation": "acme"},
            })
        );
        assert_eq!(subject_kind(&principal(true)), ("User", "alice"));
    }

    #[test]
    fn an_autonomous_agent_is_a_client() {
        assert_eq!(
            subject(&principal(false)),
            json!({"type": "Client", "id": "claude"})
        );
        assert_eq!(subject_kind(&principal(false)), ("Client", "claude"));
    }

    #[test]
    fn a_tool_resource_is_server_slash_tool_with_annotations() {
        let annotations = json!({"readOnlyHint": true});
        assert_eq!(
            resource(
                Operation::CallTool,
                "autharie",
                "list_deployments",
                &annotations
            ),
            json!({
                "type": "Tool",
                "id": "autharie/list_deployments",
                "properties": {
                    "server": "autharie",
                    "tool": "list_deployments",
                    "annotations": {"readOnlyHint": true},
                },
            })
        );
    }

    #[test]
    fn resources_and_prompts_use_their_own_types() {
        let none = json!({});
        assert_eq!(
            resource(Operation::ReadResource, "s", "file:///a", &none)["type"],
            "Resource"
        );
        assert_eq!(
            resource(Operation::GetPrompt, "s", "p", &none)["type"],
            "Prompt"
        );
        assert_eq!(Operation::ReadResource.action(), "read_resource");
        assert_eq!(Operation::GetPrompt.action(), "get_prompt");
        assert_eq!(Operation::CallTool.action(), "call_tool");
    }

    #[test]
    fn the_context_carries_the_agent_the_act_chain_and_only_primitive_arguments() {
        let arguments = json!({"s": "x", "n": 3, "b": true, "o": {"k": 1}, "a": [1], "z": null});
        let context = context(
            &principal(true),
            arguments.as_object().unwrap(),
            "2026-10-22T10:00:00Z",
        );
        assert_eq!(
            context,
            json!({
                "agent": "claude",
                "delegated": true,
                "act": {"sub": "gw"},
                "arguments": {"s": "x", "n": 3, "b": true},
                "time": "2026-10-22T10:00:00Z",
            })
        );
    }

    #[test]
    fn a_token_without_act_maps_to_a_null_act() {
        let mut p = principal(false);
        p.act = None;
        let c = context(&p, &Map::new(), "t");
        assert_eq!(c["act"], Value::Null);
        assert_eq!(c["delegated"], json!(false));
    }

    #[test]
    fn the_policies_that_decided_come_from_reason_admin() {
        let wire: WireDecision = serde_json::from_value(
            json!({"decision": true, "context": {"reason_admin": {"policies": ["p1", "p2"]}}}),
        )
        .unwrap();
        assert_eq!(
            Decision::from(wire),
            Decision {
                allowed: true,
                policies: vec!["p1".into(), "p2".into()]
            }
        );
        let bare: WireDecision = serde_json::from_value(json!({"decision": false})).unwrap();
        assert_eq!(Decision::from(bare), Decision::deny());
    }
}
