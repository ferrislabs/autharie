use std::sync::Mutex;
use std::time::Instant;

use kube::Client;
use reqwest::Method;
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::smtp::ports::FerriskeySmtpPort;
use crate::domain::smtp::{SmtpError, SmtpWrite};

use super::ferriskey_admin::{
    AdminCredentials, api_base_url, name_and_namespace, read_credentials,
};
use super::ferriskey_webhooks::{ApiFailure, ApiRequest, FerriskeyWebhookApi, TokenCache};

fn into_error(failure: ApiFailure) -> SmtpError {
    match failure {
        ApiFailure::Unauthorized => SmtpError::Retry {
            message: "FerrisKey refused the admin token".to_string(),
        },
        ApiFailure::Retry(message) => SmtpError::Retry { message },
        ApiFailure::Rejected { message, .. } => SmtpError::Rejected { message },
    }
}

fn config_path(realm: &str) -> Result<String, SmtpError> {
    let realm =
        super::ferriskey_webhooks::realm_segment(realm).map_err(|error| SmtpError::Rejected {
            message: error.message().to_string(),
        })?;
    Ok(format!("/realms/{realm}/smtp-config"))
}

pub fn put_request(realm: &str, write: &SmtpWrite<'_>) -> Result<ApiRequest, SmtpError> {
    Ok(
        ApiRequest::new(Method::PUT, config_path(realm)?).body(json!({
            "host": write.host,
            "port": write.port,
            "encryption": write.encryption,
            "username": write.username,
            "password": write.password,
            "from_email": write.from_email,
            "from_name": write.from_name,
        })),
    )
}

pub fn delete_request(realm: &str) -> Result<ApiRequest, SmtpError> {
    Ok(ApiRequest::new(Method::DELETE, config_path(realm)?))
}

fn written(result: Result<Value, ApiFailure>) -> Result<(), SmtpError> {
    result.map(|_| ()).map_err(into_error)
}

/// A realm with no mail provider is the state a delete is after.
fn deleted(result: Result<Value, ApiFailure>) -> Result<(), SmtpError> {
    match result {
        Ok(_) | Err(ApiFailure::Rejected { status: 404, .. }) => Ok(()),
        Err(failure) => Err(into_error(failure)),
    }
}

pub struct KubeFerriskeySmtp {
    client: Client,
    api: FerriskeyWebhookApi,
    tokens: Mutex<TokenCache>,
}

impl KubeFerriskeySmtp {
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

impl FerriskeySmtpPort for KubeFerriskeySmtp {
    async fn put_config(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        write: &SmtpWrite<'_>,
    ) -> Result<(), SmtpError> {
        written(self.call(instance, &put_request(realm, write)?).await)
    }

    async fn delete_config(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> Result<(), SmtpError> {
        deleted(self.call(instance, &delete_request(realm)?).await)
    }
}

#[cfg(test)]
mod tests {
    use autharie_crds::v1alpha::identity_instance::SmtpEncryption;

    use super::*;
    use crate::infrastructure::test_support::serve;

    fn write(encryption: SmtpEncryption) -> SmtpWrite<'static> {
        SmtpWrite {
            host: "smtp.example.com",
            port: 465,
            encryption,
            username: "mailer",
            password: "s3cret",
            from_email: "no-reply@example.com",
            from_name: "Acme",
        }
    }

    #[test]
    fn the_put_carries_every_field_in_ferriskeys_names() {
        let request = put_request("acme", &write(SmtpEncryption::Tls)).unwrap();

        assert_eq!(request.method, Method::PUT);
        assert_eq!(request.path, "/realms/acme/smtp-config");
        assert_eq!(
            request.body,
            Some(json!({
                "host": "smtp.example.com",
                "port": 465,
                "encryption": "tls",
                "username": "mailer",
                "password": "s3cret",
                "from_email": "no-reply@example.com",
                "from_name": "Acme",
            }))
        );
    }

    #[test]
    fn encryption_is_sent_in_lowercase() {
        for (encryption, spelled) in [
            (SmtpEncryption::Tls, "tls"),
            (SmtpEncryption::Starttls, "starttls"),
            (SmtpEncryption::None, "none"),
        ] {
            let body = put_request("acme", &write(encryption))
                .unwrap()
                .body
                .unwrap();
            assert_eq!(body["encryption"], spelled);
        }
    }

    #[test]
    fn the_delete_has_no_body() {
        let request = delete_request("acme").unwrap();

        assert_eq!(request.method, Method::DELETE);
        assert_eq!(request.path, "/realms/acme/smtp-config");
        assert_eq!(request.body, None);
    }

    #[test]
    fn a_realm_that_could_change_the_path_is_refused() {
        for realm in ["", "..", "a/b", "a?b", "a b"] {
            assert!(
                matches!(delete_request(realm), Err(SmtpError::Rejected { .. })),
                "{realm:?}"
            );
            assert!(put_request(realm, &write(SmtpEncryption::Tls)).is_err());
        }
    }

    #[tokio::test]
    async fn the_put_goes_out_as_json_with_the_token() {
        let (url, server) = serve(vec![(200, "{}")]);
        let request = put_request("acme", &write(SmtpEncryption::Starttls)).unwrap();

        let result = FerriskeyWebhookApi::new()
            .execute(&url, "fake-token", &request)
            .await;

        assert_eq!(written(result), Ok(()));
        let seen = server.join().unwrap();
        let head = seen[0].head.to_ascii_lowercase();
        assert!(seen[0].head.starts_with("PUT /realms/acme/smtp-config "));
        assert!(head.contains("authorization: bearer fake-token"));
        assert!(head.contains("content-type: application/json"));
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(body["encryption"], "starttls");
        assert_eq!(body["password"], "s3cret");
        assert_eq!(body["from_email"], "no-reply@example.com");
        assert_eq!(body["port"], 465);
    }

    #[tokio::test]
    async fn the_delete_goes_out_without_a_body() {
        let (url, server) = serve(vec![(204, "")]);
        let request = delete_request("acme").unwrap();

        let result = FerriskeyWebhookApi::new()
            .execute(&url, "t", &request)
            .await;

        assert_eq!(deleted(result), Ok(()));
        let seen = server.join().unwrap();
        assert!(seen[0].head.starts_with("DELETE /realms/acme/smtp-config "));
        assert_eq!(seen[0].body, "");
    }

    #[tokio::test]
    async fn statuses_become_the_outcome_a_pass_acts_on() {
        let (url, server) = serve(vec![
            (503, "{}"),
            (408, "{}"),
            (429, "{}"),
            (400, r#"{"message":"invalid host"}"#),
            (422, r#"{"message":"bad port"}"#),
            (401, "{}"),
        ]);
        let api = FerriskeyWebhookApi::new();
        let request = put_request("acme", &write(SmtpEncryption::Tls)).unwrap();

        let mut results = Vec::new();
        for _ in 0..6 {
            results.push(written(api.execute(&url, "t", &request).await));
        }

        for temporary in [0, 1, 2, 5] {
            assert!(
                matches!(&results[temporary], Err(SmtpError::Retry { .. })),
                "{temporary}"
            );
        }
        assert_eq!(
            results[3],
            Err(SmtpError::Rejected {
                message: "FerrisKey answered 400: invalid host".to_string()
            })
        );
        assert!(matches!(&results[4], Err(SmtpError::Rejected { .. })));
        server.join().unwrap();
    }

    #[test]
    fn deleting_what_is_already_gone_is_done() {
        assert_eq!(deleted(Ok(Value::Null)), Ok(()));
        assert_eq!(
            deleted(Err(ApiFailure::Rejected {
                status: 404,
                message: "FerrisKey answered 404".to_string()
            })),
            Ok(())
        );
    }

    #[test]
    fn any_other_failure_of_a_delete_is_kept() {
        assert!(matches!(
            deleted(Err(ApiFailure::Retry(
                "FerrisKey is unreachable".to_string()
            ))),
            Err(SmtpError::Retry { .. })
        ));
        assert!(matches!(
            deleted(Err(ApiFailure::Rejected {
                status: 403,
                message: "FerrisKey answered 403".to_string()
            })),
            Err(SmtpError::Rejected { .. })
        ));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_temporary_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let request = delete_request("acme").unwrap();

        let result = FerriskeyWebhookApi::new()
            .execute(&url, "t", &request)
            .await;

        assert!(matches!(deleted(result), Err(SmtpError::Retry { .. })));
    }
}
