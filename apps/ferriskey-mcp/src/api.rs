use std::{fmt, time::Duration};

use reqwest::{StatusCode, redirect::Policy};
use serde_json::Value;

use crate::upstream::Instance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub message: String,
}

impl Fault {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Fault {}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    pub async fn get(
        &self,
        instance: &Instance,
        token: &str,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<Value, Fault> {
        let request = self.http.get(instance.url(segments)).query(query);
        self.send(request, token).await
    }

    pub async fn post(
        &self,
        instance: &Instance,
        token: &str,
        segments: &[&str],
        body: Option<&Value>,
    ) -> Result<Value, Fault> {
        let request = self.http.post(instance.url(segments));
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        self.send(request, token).await
    }

    pub async fn put(
        &self,
        instance: &Instance,
        token: &str,
        segments: &[&str],
        body: &Value,
    ) -> Result<Value, Fault> {
        let request = self.http.put(instance.url(segments)).json(body);
        self.send(request, token).await
    }

    pub async fn delete(
        &self,
        instance: &Instance,
        token: &str,
        segments: &[&str],
    ) -> Result<Value, Fault> {
        let request = self.http.delete(instance.url(segments));
        self.send(request, token).await
    }

    async fn send(&self, request: reqwest::RequestBuilder, token: &str) -> Result<Value, Fault> {
        let response = request
            .bearer_auth(token)
            .send()
            .await
            .map_err(unreachable_api)?;
        let status = response.status();
        if !status.is_success() {
            return Err(refusal(status, response).await);
        }
        let text = response.text().await.map_err(unreachable_api)?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text)
            .map_err(|_| Fault::new("the FerrisKey API answered with a body that is not JSON"))
    }
}

fn unreachable_api(error: reqwest::Error) -> Fault {
    tracing::warn!(%error, "FerrisKey API call failed");
    Fault::new("the FerrisKey API is unreachable")
}

async fn refusal(status: StatusCode, response: reqwest::Response) -> Fault {
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "no detail given".to_owned());
    Fault::new(format!("FerrisKey API {}: {message}", status.as_u16()))
}
