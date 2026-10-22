use kube::Client;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

use crate::domain::identity_instance::theme::ThemeError;
use crate::domain::ports::InstanceThemePort;
use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use super::ferriskey_admin::{
    ADMIN_REALM, AdminCredentials, api_base_url, error_message, http_client, name_and_namespace,
    read_credentials, request_token, unreachable_message,
};

pub struct FerriskeyThemeClient {
    http: reqwest::Client,
}

impl Default for FerriskeyThemeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl FerriskeyThemeClient {
    pub fn new() -> Self {
        Self {
            http: http_client(),
        }
    }

    pub async fn put_theme(
        &self,
        base_url: &str,
        credentials: &AdminCredentials<'_>,
        config: &Value,
    ) -> Result<(), ThemeError> {
        let token = request_token(&self.http, base_url, credentials)
            .await
            .map_err(|message| ThemeError::Retry { message })?;
        let body = json!({ "config": config }).to_string();

        let response = self
            .http
            .put(format!("{base_url}/realms/{ADMIN_REALM}/portal/theme"))
            .bearer_auth(token.value)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| ThemeError::Retry {
                message: unreachable_message(&error),
            })?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }

        let message = error_message(status.as_u16(), &response.text().await.unwrap_or_default());
        if status.is_server_error() || matches!(status.as_u16(), 408 | 429) {
            Err(ThemeError::Retry { message })
        } else {
            Err(ThemeError::Rejected { message })
        }
    }
}

pub struct KubeInstanceTheme {
    client: Client,
    admin: FerriskeyThemeClient,
}

impl KubeInstanceTheme {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            admin: FerriskeyThemeClient::new(),
        }
    }
}

impl InstanceThemePort for KubeInstanceTheme {
    async fn put_theme(
        &self,
        instance: &IdentityInstance,
        config: &Value,
    ) -> Result<(), ThemeError> {
        let Some((name, namespace)) = name_and_namespace(instance) else {
            return Err(ThemeError::Retry {
                message: "the instance has no name or namespace yet".to_string(),
            });
        };

        let (username, password) = read_credentials(&self.client, name, namespace)
            .await
            .map_err(|message| ThemeError::Retry { message })?;
        self.admin
            .put_theme(
                &api_base_url(name, namespace),
                &AdminCredentials {
                    username: &username,
                    password: &password,
                },
                config,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use super::*;

    struct Recorded {
        head: String,
        body: String,
    }

    fn serve(script: Vec<(u16, &'static str)>) -> (String, JoinHandle<Vec<Recorded>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in script {
                let (mut stream, _) = listener.accept().unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 1024];
                let split = loop {
                    let read = stream.read(&mut buf).unwrap();
                    raw.extend_from_slice(&buf[..read]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                    assert!(read > 0, "connection closed before the headers ended");
                };
                let head = String::from_utf8_lossy(&raw[..split]).to_string();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while raw.len() < split + length {
                    let read = stream.read(&mut buf).unwrap();
                    raw.extend_from_slice(&buf[..read]);
                }
                let request_body = String::from_utf8_lossy(&raw[split..split + length]).to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                seen.push(Recorded {
                    head,
                    body: request_body,
                });
            }
            seen
        });
        (url, handle)
    }

    const TOKEN_OK: (u16, &str) = (200, r#"{"access_token":"fake-token"}"#);

    fn credentials() -> AdminCredentials<'static> {
        AdminCredentials {
            username: "fake-user",
            password: "fake-pass",
        }
    }

    #[tokio::test]
    async fn sends_a_token_request_then_the_theme() {
        let (url, server) = serve(vec![TOKEN_OK, (200, "{}")]);
        let config = json!({"colors": {"primaryButton": "#112233"}});

        FerriskeyThemeClient::new()
            .put_theme(&url, &credentials(), &config)
            .await
            .unwrap();

        let seen = server.join().unwrap();
        assert!(
            seen[0]
                .head
                .starts_with("POST /realms/master/protocol/openid-connect/token ")
        );
        assert_eq!(
            seen[0].body,
            "grant_type=password&client_id=admin-cli&username=fake-user&password=fake-pass"
        );
        assert!(seen[1].head.starts_with("PUT /realms/master/portal/theme "));
        assert!(
            seen[1]
                .head
                .to_ascii_lowercase()
                .contains("authorization: bearer fake-token")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&seen[1].body).unwrap(),
            json!({"config": {"colors": {"primaryButton": "#112233"}}})
        );
    }

    #[tokio::test]
    async fn a_503_is_a_retry_and_the_next_attempt_succeeds() {
        let (url, server) = serve(vec![TOKEN_OK, (503, "{}"), TOKEN_OK, (200, "{}")]);
        let client = FerriskeyThemeClient::new();
        let config = json!({});

        let first = client.put_theme(&url, &credentials(), &config).await;
        assert!(matches!(first, Err(ThemeError::Retry { .. })));
        client
            .put_theme(&url, &credentials(), &config)
            .await
            .unwrap();
        server.join().unwrap();
    }

    #[tokio::test]
    async fn a_4xx_is_rejected_with_the_message_from_ferriskey() {
        let (url, server) = serve(vec![
            TOKEN_OK,
            (
                400,
                r#"{"code":"E","status":400,"reason":"r","message":"invalid color"}"#,
            ),
        ]);

        let result = FerriskeyThemeClient::new()
            .put_theme(&url, &credentials(), &json!({}))
            .await;

        assert_eq!(
            result,
            Err(ThemeError::Rejected {
                message: "FerrisKey answered 400: invalid color".to_string()
            })
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn a_token_failure_is_a_retry_and_sends_no_theme() {
        let (url, server) = serve(vec![(401, r#"{"message":"bad credentials"}"#)]);

        let result = FerriskeyThemeClient::new()
            .put_theme(&url, &credentials(), &json!({}))
            .await;

        let Err(ThemeError::Retry { message }) = result else {
            panic!("expected a retry");
        };
        assert!(!message.contains("fake-pass"));
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);

        let result = FerriskeyThemeClient::new()
            .put_theme(&url, &credentials(), &json!({}))
            .await;

        assert!(matches!(result, Err(ThemeError::Retry { .. })));
    }

    #[test]
    fn the_api_is_reached_through_its_service() {
        assert_eq!(
            api_base_url("acme", "tenant-a"),
            "http://acme-api.tenant-a.svc:3333/api"
        );
    }
}
