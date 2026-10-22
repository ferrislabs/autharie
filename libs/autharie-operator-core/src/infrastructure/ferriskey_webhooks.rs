use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kube::Client;
use reqwest::Method;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::webhooks::ports::FerriskeyWebhookPort;
use crate::domain::webhooks::{RemoteWebhook, WebhookError, WebhookWrite};

use super::ferriskey_admin::{
    ADMIN_REALM, AdminCredentials, api_base_url, error_message, http_client, name_and_namespace,
    read_credentials, request_token, unreachable_message,
};

const PAGE_LIMIT: u32 = 100;
const MAX_PAGES: u32 = 100;

/// Refreshed this long before FerrisKey says it expires, so a call never
/// starts with a token about to die.
const TOKEN_MARGIN: Duration = Duration::from_secs(30);
const DEFAULT_TOKEN_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
pub struct ApiRequest {
    pub method: Method,
    pub path: String,
    pub query: Vec<(&'static str, String)>,
    pub body: Option<Value>,
}

impl ApiRequest {
    fn new(method: Method, path: String) -> Self {
        Self {
            method,
            path,
            query: Vec::new(),
            body: None,
        }
    }

    fn query(mut self, key: &'static str, value: impl ToString) -> Self {
        self.query.push((key, value.to_string()));
        self
    }

    fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiFailure {
    Unauthorized,
    Retry(String),
    Rejected { status: u16, message: String },
}

impl ApiFailure {
    fn into_error(self) -> WebhookError {
        match self {
            Self::Unauthorized => WebhookError::Retry {
                message: "FerrisKey refused the admin token".to_string(),
            },
            Self::Retry(message) => WebhookError::Retry { message },
            Self::Rejected { message, .. } => WebhookError::Rejected { message },
        }
    }
}

pub struct FerriskeyWebhookApi {
    http: reqwest::Client,
}

impl Default for FerriskeyWebhookApi {
    fn default() -> Self {
        Self::new()
    }
}

impl FerriskeyWebhookApi {
    pub fn new() -> Self {
        Self {
            http: http_client(),
        }
    }

    pub async fn token(
        &self,
        base_url: &str,
        credentials: &AdminCredentials<'_>,
    ) -> Result<(String, Duration), ApiFailure> {
        let token = request_token(&self.http, base_url, credentials)
            .await
            .map_err(ApiFailure::Retry)?;
        let lifetime = token
            .expires_in
            .map_or(DEFAULT_TOKEN_LIFETIME, Duration::from_secs);
        Ok((token.value, lifetime))
    }

    pub async fn execute(
        &self,
        base_url: &str,
        token: &str,
        request: &ApiRequest,
    ) -> Result<Value, ApiFailure> {
        let mut builder = self
            .http
            .request(
                request.method.clone(),
                format!("{base_url}{}", request.path),
            )
            .bearer_auth(token);
        if !request.query.is_empty() {
            builder = builder.query(&request.query);
        }
        if let Some(body) = &request.body {
            builder = builder
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }

        let response = builder
            .send()
            .await
            .map_err(|error| ApiFailure::Retry(unreachable_message(&error)))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if status.is_success() {
            return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        if status.as_u16() == 401 {
            return Err(ApiFailure::Unauthorized);
        }
        let message = error_message(status.as_u16(), &text);
        if status.is_server_error() || matches!(status.as_u16(), 408 | 429) {
            Err(ApiFailure::Retry(message))
        } else {
            Err(ApiFailure::Rejected {
                status: status.as_u16(),
                message,
            })
        }
    }
}

#[derive(Default)]
struct TokenCache {
    entries: HashMap<String, (String, Instant)>,
}

impl TokenCache {
    fn get(&self, key: &str, now: Instant) -> Option<String> {
        self.entries
            .get(key)
            .filter(|(_, expires)| *expires > now)
            .map(|(token, _)| token.clone())
    }

    fn put(&mut self, key: &str, token: String, lifetime: Duration, now: Instant) {
        let usable = lifetime.saturating_sub(TOKEN_MARGIN);
        self.entries.insert(key.to_string(), (token, now + usable));
    }

    fn forget(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

/// Realm names are slugs; anything else would change the path it lands in.
fn realm_segment(realm: &str) -> Result<&str, WebhookError> {
    let slug = !realm.is_empty()
        && realm
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if slug && realm != "." && realm != ".." {
        Ok(realm)
    } else {
        Err(WebhookError::Rejected {
            message: format!("{realm:?} is not a valid realm name"),
        })
    }
}

fn webhook_path(realm: &str, ferriskey_id: &str) -> Result<String, WebhookError> {
    let realm = realm_segment(realm)?;
    realm_segment(ferriskey_id)?;
    Ok(format!("/realms/{realm}/webhooks/{ferriskey_id}"))
}

fn list_realms_request(page: u32) -> ApiRequest {
    ApiRequest::new(
        Method::GET,
        format!("/realms/{ADMIN_REALM}/users/@me/realms"),
    )
    .query("page", page)
    .query("limit", PAGE_LIMIT)
}

fn list_webhooks_request(realm: &str, page: u32) -> Result<ApiRequest, WebhookError> {
    let realm = realm_segment(realm)?;
    Ok(
        ApiRequest::new(Method::GET, format!("/realms/{realm}/webhooks"))
            .query("name", crate::domain::webhooks::MANAGED_PREFIX)
            .query("page", page)
            .query("limit", PAGE_LIMIT),
    )
}

fn write_body(write: &WebhookWrite) -> Value {
    let mut body = json!({
        "name": write.name,
        "description": write.description,
        "endpoint": write.endpoint,
        "subscribers": write.events,
    });
    if let Some(headers) = &write.headers {
        body["headers"] = json!(headers);
    }
    body
}

fn create_request(realm: &str, write: &WebhookWrite) -> Result<ApiRequest, WebhookError> {
    let realm = realm_segment(realm)?;
    let mut body = write_body(write);
    body["headers"] = json!(write.headers.clone().unwrap_or_default());
    Ok(ApiRequest::new(Method::POST, format!("/realms/{realm}/webhooks")).body(body))
}

fn update_request(
    realm: &str,
    ferriskey_id: &str,
    write: &WebhookWrite,
) -> Result<ApiRequest, WebhookError> {
    Ok(ApiRequest::new(Method::PUT, webhook_path(realm, ferriskey_id)?).body(write_body(write)))
}

fn malformed(what: &str) -> WebhookError {
    WebhookError::Retry {
        message: format!("FerrisKey answered {what} in a shape the operator does not know"),
    }
}

/// The names in a page, and the page after it when there is one.
fn parse_names(body: &Value) -> Result<(Vec<String>, Option<u32>), WebhookError> {
    let data = body["data"].as_array().ok_or_else(|| malformed("a list"))?;
    let names = data
        .iter()
        .filter_map(|item| item["name"].as_str().map(ToOwned::to_owned))
        .collect();
    Ok((names, next_page(body)))
}

fn next_page(body: &Value) -> Option<u32> {
    body["metadata"]["next_page"]
        .as_u64()
        .and_then(|page| u32::try_from(page).ok())
}

fn parse_webhook(value: &Value) -> Result<RemoteWebhook, WebhookError> {
    let ferriskey_id = value["id"]
        .as_str()
        .ok_or_else(|| malformed("a webhook"))?
        .to_string();
    let endpoint = value["endpoint"]
        .as_str()
        .ok_or_else(|| malformed("a webhook"))?
        .to_string();
    let events = value["subscribers"]
        .as_array()
        .map(|subscribers| {
            subscribers
                .iter()
                .filter_map(|subscriber| subscriber["name"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(RemoteWebhook {
        ferriskey_id,
        name: value["name"].as_str().map(ToOwned::to_owned),
        description: value["description"].as_str().map(ToOwned::to_owned),
        endpoint,
        events,
    })
}

fn parse_webhooks(body: &Value) -> Result<(Vec<RemoteWebhook>, Option<u32>), WebhookError> {
    let data = body["data"].as_array().ok_or_else(|| malformed("a list"))?;
    let webhooks = data
        .iter()
        .map(parse_webhook)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((webhooks, next_page(body)))
}

fn parse_secret(body: &Value) -> Result<String, WebhookError> {
    body["secret"]
        .as_str()
        .filter(|secret| !secret.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| malformed("a rotation without a secret"))
}

pub struct KubeFerriskeyWebhooks {
    client: Client,
    api: FerriskeyWebhookApi,
    tokens: Mutex<TokenCache>,
}

impl KubeFerriskeyWebhooks {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            api: FerriskeyWebhookApi::new(),
            tokens: Mutex::new(TokenCache::default()),
        }
    }

    async fn token(
        &self,
        base_url: &str,
        name: &str,
        namespace: &str,
    ) -> Result<String, ApiFailure> {
        if let Some(token) = self
            .tokens
            .lock()
            .ok()
            .and_then(|cache| cache.get(base_url, Instant::now()))
        {
            return Ok(token);
        }

        let (username, password) = read_credentials(&self.client, name, namespace)
            .await
            .map_err(ApiFailure::Retry)?;
        let (token, lifetime) = self
            .api
            .token(
                base_url,
                &AdminCredentials {
                    username: &username,
                    password: &password,
                },
            )
            .await?;
        if let Ok(mut cache) = self.tokens.lock() {
            cache.put(base_url, token.clone(), lifetime, Instant::now());
        }
        Ok(token)
    }

    async fn call(
        &self,
        instance: &IdentityInstance,
        request: &ApiRequest,
    ) -> Result<Value, ApiFailure> {
        let Some((name, namespace)) = name_and_namespace(instance) else {
            return Err(ApiFailure::Retry(
                "the instance has no name or namespace yet".to_string(),
            ));
        };
        let base_url = api_base_url(name, namespace);

        let token = self.token(&base_url, name, namespace).await?;
        match self.api.execute(&base_url, &token, request).await {
            Err(ApiFailure::Unauthorized) => {
                if let Ok(mut cache) = self.tokens.lock() {
                    cache.forget(&base_url);
                }
                let token = self.token(&base_url, name, namespace).await?;
                self.api.execute(&base_url, &token, request).await
            }
            other => other,
        }
    }

    async fn pages<T>(
        &self,
        instance: &IdentityInstance,
        request: impl Fn(u32) -> Result<ApiRequest, WebhookError>,
        parse: impl Fn(&Value) -> Result<(Vec<T>, Option<u32>), WebhookError>,
    ) -> Result<Vec<T>, WebhookError> {
        let mut all = Vec::new();
        let mut page = 1;
        for _ in 0..MAX_PAGES {
            let body = self
                .call(instance, &request(page)?)
                .await
                .map_err(ApiFailure::into_error)?;
            let (mut items, next) = parse(&body)?;
            all.append(&mut items);
            match next {
                Some(next) if next > page => page = next,
                _ => break,
            }
        }
        Ok(all)
    }
}

impl FerriskeyWebhookPort for KubeFerriskeyWebhooks {
    async fn list_realms(&self, instance: &IdentityInstance) -> Result<Vec<String>, WebhookError> {
        self.pages(instance, |page| Ok(list_realms_request(page)), parse_names)
            .await
    }

    async fn list_managed_webhooks(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> Result<Vec<RemoteWebhook>, WebhookError> {
        self.pages(
            instance,
            |page| list_webhooks_request(realm, page),
            parse_webhooks,
        )
        .await
    }

    async fn create_webhook(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        write: &WebhookWrite,
    ) -> Result<RemoteWebhook, WebhookError> {
        let body = self
            .call(instance, &create_request(realm, write)?)
            .await
            .map_err(ApiFailure::into_error)?;
        parse_webhook(&body["data"])
    }

    async fn update_webhook(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
        write: &WebhookWrite,
    ) -> Result<(), WebhookError> {
        self.call(instance, &update_request(realm, ferriskey_id, write)?)
            .await
            .map(|_| ())
            .map_err(ApiFailure::into_error)
    }

    async fn delete_webhook(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
    ) -> Result<(), WebhookError> {
        let request = ApiRequest::new(Method::DELETE, webhook_path(realm, ferriskey_id)?);
        match self.call(instance, &request).await {
            Ok(_) | Err(ApiFailure::Rejected { status: 404, .. }) => Ok(()),
            Err(failure) => Err(failure.into_error()),
        }
    }

    async fn rotate_secret(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
    ) -> Result<String, WebhookError> {
        let request = ApiRequest::new(
            Method::POST,
            format!("{}/secret/rotate", webhook_path(realm, ferriskey_id)?),
        );
        let body = self
            .call(instance, &request)
            .await
            .map_err(ApiFailure::into_error)?;
        parse_secret(&body)
    }

    async fn retry_delivery(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        ferriskey_id: &str,
        delivery_id: &str,
    ) -> Result<(), WebhookError> {
        realm_segment(delivery_id)?;
        let request = ApiRequest::new(
            Method::POST,
            format!(
                "{}/deliveries/{delivery_id}/retry",
                webhook_path(realm, ferriskey_id)?
            ),
        );
        self.call(instance, &request)
            .await
            .map(|_| ())
            .map_err(ApiFailure::into_error)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::infrastructure::test_support::serve;

    fn write(headers: Option<BTreeMap<String, String>>) -> WebhookWrite {
        WebhookWrite {
            name: "autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11".to_string(),
            description: "Audit — to the SIEM".to_string(),
            endpoint: "https://example.com/hook".to_string(),
            events: vec!["user.created".to_string()],
            headers,
        }
    }

    #[test]
    fn creating_always_sends_a_headers_map_and_updating_only_when_asked() {
        let create = create_request("master", &write(None)).unwrap();
        let body = create.body.unwrap();
        assert_eq!(body["headers"], json!({}));
        assert_eq!(body["subscribers"], json!(["user.created"]));
        assert_eq!(
            body["name"],
            "autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11"
        );

        let keep = update_request("master", "f1", &write(None)).unwrap();
        assert!(keep.body.unwrap().get("headers").is_none());

        let headers = BTreeMap::from([("X-Env".to_string(), "prod".to_string())]);
        let replace = update_request("master", "f1", &write(Some(headers))).unwrap();
        assert_eq!(replace.method, Method::PUT);
        assert_eq!(replace.path, "/realms/master/webhooks/f1");
        assert_eq!(replace.body.unwrap()["headers"], json!({"X-Env": "prod"}));
    }

    #[test]
    fn names_that_could_change_the_path_are_refused() {
        for realm in ["", "..", "a/b", "a?b", "a b", "master/../x"] {
            assert!(realm_segment(realm).is_err(), "{realm:?}");
        }
        assert!(realm_segment("my-realm_1.x").is_ok());
        assert!(list_webhooks_request("a/b", 1).is_err());
        assert!(webhook_path("master", "../secret").is_err());
    }

    #[test]
    fn a_webhook_is_read_with_its_subscribers_and_no_secret() {
        let parsed = parse_webhooks(&json!({
            "data": [{
                "id": "f1",
                "endpoint": "https://example.com",
                "name": "autharie:x",
                "description": "d",
                "subscribers": [
                    {"id": "s1", "name": "user.created", "webhook_id": "f1"},
                    {"id": "s2", "name": "user.deleted", "webhook_id": "f1"}
                ],
                "created_at": "2026-10-22T00:00:00Z"
            }, {
                "id": "f2", "endpoint": "https://other", "name": null, "description": null,
                "subscribers": []
            }],
            "metadata": {"page": 1, "next_page": 2}
        }))
        .unwrap();

        assert_eq!(parsed.1, Some(2));
        assert_eq!(parsed.0[0].events, vec!["user.created", "user.deleted"]);
        assert_eq!(parsed.0[0].name.as_deref(), Some("autharie:x"));
        assert_eq!(parsed.0[1].name, None);
    }

    #[test]
    fn answers_in_an_unknown_shape_are_a_retry_not_a_guess() {
        assert!(matches!(
            parse_webhooks(&json!({"nope": 1})),
            Err(WebhookError::Retry { .. })
        ));
        assert!(matches!(
            parse_secret(&json!({})),
            Err(WebhookError::Retry { .. })
        ));
        assert!(matches!(
            parse_webhook(&json!({"id": "f"})),
            Err(WebhookError::Retry { .. })
        ));
    }

    #[test]
    fn realm_names_and_the_last_page_are_read() {
        let (names, next) = parse_names(&json!({
            "data": [{"id": "1", "name": "master"}, {"id": "2", "name": "acme"}],
            "metadata": {"next_page": null}
        }))
        .unwrap();

        assert_eq!(names, vec!["master", "acme"]);
        assert_eq!(next, None);
    }

    #[test]
    fn a_token_is_reused_until_shortly_before_it_expires() {
        let mut cache = TokenCache::default();
        let start = Instant::now();
        cache.put("k", "t".to_string(), Duration::from_secs(300), start);

        assert_eq!(cache.get("k", start).as_deref(), Some("t"));
        assert_eq!(
            cache.get("k", start + Duration::from_secs(269)).as_deref(),
            Some("t")
        );
        assert_eq!(cache.get("k", start + Duration::from_secs(271)), None);
        cache.forget("k");
        assert_eq!(cache.get("k", start), None);
    }

    #[test]
    fn a_short_lived_token_is_not_cached_at_all() {
        let mut cache = TokenCache::default();
        let start = Instant::now();
        cache.put("k", "t".to_string(), Duration::from_secs(10), start);

        assert_eq!(cache.get("k", start), None);
    }

    #[tokio::test]
    async fn a_request_carries_the_token_the_query_and_the_body() {
        let (url, server) = serve(vec![(200, r#"{"data":{"id":"f1"}}"#)]);
        let request = create_request("master", &write(None)).unwrap();

        let body = FerriskeyWebhookApi::new()
            .execute(&url, "fake-token", &request)
            .await
            .unwrap();

        assert_eq!(body["data"]["id"], "f1");
        let seen = server.join().unwrap();
        assert!(seen[0].head.starts_with("POST /realms/master/webhooks "));
        assert!(
            seen[0]
                .head
                .to_ascii_lowercase()
                .contains("authorization: bearer fake-token")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&seen[0].body).unwrap()["endpoint"],
            "https://example.com/hook"
        );
    }

    #[tokio::test]
    async fn the_listing_asks_for_managed_names_a_page_at_a_time() {
        let (url, server) = serve(vec![(200, r#"{"data":[],"metadata":{"next_page":null}}"#)]);
        let request = list_webhooks_request("acme", 2).unwrap();

        FerriskeyWebhookApi::new()
            .execute(&url, "t", &request)
            .await
            .unwrap();

        let seen = server.join().unwrap();
        assert!(
            seen[0]
                .head
                .starts_with("GET /realms/acme/webhooks?name=autharie%3A&page=2&limit=100 ")
        );
    }

    #[tokio::test]
    async fn statuses_map_to_unauthorized_retry_or_rejected() {
        let (url, server) = serve(vec![
            (401, "{}"),
            (503, "{}"),
            (429, "{}"),
            (404, r#"{"message":"webhook not found"}"#),
            (204, ""),
        ]);
        let api = FerriskeyWebhookApi::new();
        let request = ApiRequest::new(Method::DELETE, "/realms/master/webhooks/f1".to_string());

        let results = [
            api.execute(&url, "t", &request).await,
            api.execute(&url, "t", &request).await,
            api.execute(&url, "t", &request).await,
            api.execute(&url, "t", &request).await,
            api.execute(&url, "t", &request).await,
        ];

        assert_eq!(results[0], Err(ApiFailure::Unauthorized));
        assert!(matches!(results[1], Err(ApiFailure::Retry(_))));
        assert!(matches!(results[2], Err(ApiFailure::Retry(_))));
        assert_eq!(
            results[3],
            Err(ApiFailure::Rejected {
                status: 404,
                message: "FerrisKey answered 404: webhook not found".to_string()
            })
        );
        assert_eq!(results[4], Ok(Value::Null));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_retry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let request = list_realms_request(1);

        let result = FerriskeyWebhookApi::new()
            .execute(&url, "t", &request)
            .await;

        assert!(matches!(result, Err(ApiFailure::Retry(_))));
    }

    #[tokio::test]
    async fn the_token_comes_with_its_lifetime() {
        let (url, server) = serve(vec![(
            200,
            r#"{"access_token":"fake-token","expires_in":300}"#,
        )]);

        let (token, lifetime) = FerriskeyWebhookApi::new()
            .token(
                &url,
                &AdminCredentials {
                    username: "u",
                    password: "p",
                },
            )
            .await
            .unwrap();

        assert_eq!(token, "fake-token");
        assert_eq!(lifetime, Duration::from_secs(300));
        server.join().unwrap();
    }

    #[test]
    fn a_failure_keeps_its_class_when_it_becomes_an_error() {
        assert!(ApiFailure::Unauthorized.into_error().is_retry());
        assert!(ApiFailure::Retry("x".into()).into_error().is_retry());
        assert!(
            !ApiFailure::Rejected {
                status: 400,
                message: "x".into()
            }
            .into_error()
            .is_retry()
        );
    }
}
