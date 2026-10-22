use std::fmt;

use reqwest::StatusCode;
use serde_json::Value;

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
    base: String,
}

impl ApiClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.into().trim_end_matches('/').to_owned(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }

    pub async fn get(
        &self,
        token: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, Fault> {
        self.send(self.http.get(self.url(path)).query(query), token)
            .await
    }

    pub async fn post(&self, token: &str, path: &str, body: &Value) -> Result<Value, Fault> {
        self.send(self.http.post(self.url(path)).json(body), token)
            .await
    }

    pub async fn delete(&self, token: &str, path: &str) -> Result<Value, Fault> {
        self.send(self.http.delete(self.url(path)), token).await
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
            .map_err(|_| Fault::new("the Autharie API answered with a body that is not JSON"))
    }
}

fn unreachable_api(error: reqwest::Error) -> Fault {
    tracing::warn!(%error, "Autharie API call failed");
    Fault::new("the Autharie API is unreachable")
}

async fn refusal(status: StatusCode, response: reqwest::Response) -> Fault {
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "no detail given".to_owned());
    Fault::new(format!("Autharie API {}: {message}", status.as_u16()))
}
