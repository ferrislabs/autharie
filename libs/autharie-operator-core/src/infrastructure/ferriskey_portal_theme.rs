use std::sync::Mutex;
use std::time::Instant;

use kube::Client;
use reqwest::Method;
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::portal_themes::PortalThemeError;
use crate::domain::portal_themes::ports::FerriskeyPortalThemePort;

use super::ferriskey_admin::{
    AdminCredentials, api_base_url, name_and_namespace, read_credentials,
};
use super::ferriskey_webhooks::{ApiFailure, ApiRequest, FerriskeyWebhookApi, TokenCache};

fn into_error(failure: ApiFailure) -> PortalThemeError {
    match failure {
        ApiFailure::Unauthorized => PortalThemeError::Retry {
            message: "FerrisKey refused the admin token".to_string(),
        },
        ApiFailure::Retry(message) => PortalThemeError::Retry { message },
        ApiFailure::Rejected { message, .. } => PortalThemeError::Rejected { message },
    }
}

pub fn put_request(realm: &str, config: &Value) -> Result<ApiRequest, PortalThemeError> {
    let realm = super::ferriskey_webhooks::realm_segment(realm).map_err(|error| {
        PortalThemeError::Rejected {
            message: error.message().to_string(),
        }
    })?;
    Ok(
        ApiRequest::new(Method::PUT, format!("/realms/{realm}/portal/theme"))
            .body(json!({ "config": config })),
    )
}

fn written(result: Result<Value, ApiFailure>) -> Result<(), PortalThemeError> {
    result.map(|_| ()).map_err(into_error)
}

pub struct KubeFerriskeyPortalTheme {
    client: Client,
    api: FerriskeyWebhookApi,
    tokens: Mutex<TokenCache>,
}

impl KubeFerriskeyPortalTheme {
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
}

impl FerriskeyPortalThemePort for KubeFerriskeyPortalTheme {
    async fn put_theme(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        config: &Value,
    ) -> Result<(), PortalThemeError> {
        written(self.call(instance, &put_request(realm, config)?).await)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::infrastructure::test_support::serve;

    fn config() -> Value {
        json!({
            "colors": { "primaryButton": "#112233" },
            "fonts": { "baseSize": 16 },
            "borders": { "radius": 8 },
            "spacing": { "page": 24 }
        })
    }

    #[test]
    fn the_put_wraps_the_config_the_way_ferriskey_reads_it() {
        let request = put_request("acme", &config()).unwrap();

        assert_eq!(request.method, Method::PUT);
        assert_eq!(request.path, "/realms/acme/portal/theme");
        assert_eq!(request.body, Some(json!({ "config": config() })));
        assert!(request.query.is_empty());
    }

    #[test]
    fn empty_tokens_are_sent_as_an_empty_object() {
        let request = put_request("master", &json!({})).unwrap();

        assert_eq!(request.path, "/realms/master/portal/theme");
        assert_eq!(request.body, Some(json!({ "config": {} })));
    }

    #[test]
    fn a_realm_that_could_change_the_path_is_refused() {
        for realm in ["", "..", "a/b", "a?b", "a b"] {
            assert!(
                matches!(
                    put_request(realm, &config()),
                    Err(PortalThemeError::Rejected { .. })
                ),
                "{realm:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_put_goes_out_as_json_with_the_token_and_the_config_as_the_body() {
        let (url, server) = serve(vec![(200, "{}")]);
        let request = put_request("acme", &config()).unwrap();

        let result = FerriskeyWebhookApi::new()
            .execute(&url, "fake-token", &request)
            .await;

        assert_eq!(written(result), Ok(()));
        let seen = server.join().unwrap();
        let head = seen[0].head.to_ascii_lowercase();
        assert!(seen[0].head.starts_with("PUT /realms/acme/portal/theme "));
        assert!(head.contains("authorization: bearer fake-token"));
        assert!(head.contains("content-type: application/json"));
        assert_eq!(
            serde_json::from_str::<Value>(&seen[0].body).unwrap(),
            json!({ "config": config() })
        );
    }

    #[tokio::test]
    async fn statuses_become_the_outcome_a_pass_acts_on() {
        let (url, server) = serve(vec![
            (503, "{}"),
            (408, "{}"),
            (429, "{}"),
            (400, r#"{"message":"invalid colour"}"#),
            (422, r#"{"message":"bad radius"}"#),
            (401, "{}"),
        ]);
        let api = FerriskeyWebhookApi::new();
        let request = put_request("acme", &config()).unwrap();

        let mut results = Vec::new();
        for _ in 0..6 {
            results.push(written(api.execute(&url, "t", &request).await));
        }

        for temporary in [0, 1, 2, 5] {
            assert!(
                matches!(&results[temporary], Err(PortalThemeError::Retry { .. })),
                "{temporary}"
            );
        }
        assert_eq!(
            results[3],
            Err(PortalThemeError::Rejected {
                message: "FerrisKey answered 400: invalid colour".to_string()
            })
        );
        assert!(matches!(
            &results[4],
            Err(PortalThemeError::Rejected { .. })
        ));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_temporary_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let request = put_request("acme", &config()).unwrap();

        let result = FerriskeyWebhookApi::new()
            .execute(&url, "t", &request)
            .await;

        assert!(matches!(
            written(result),
            Err(PortalThemeError::Retry { .. })
        ));
    }
}
