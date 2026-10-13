use std::time::Duration;

use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde_json::{Value, json};

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::RealmError;
use crate::domain::ports::RealmPort;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const ADMIN_REALM: &str = "master";
const ADMIN_CLIENT: &str = "admin-cli";
const API_PORT: u16 = 3333;
const API_ROOT_PATH: &str = "/api";
const REDACTED: &str = "[redacted]";

pub struct AdminCredentials {
    username: String,
    password: String,
}

impl AdminCredentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    fn redact(&self, text: &str) -> String {
        if self.password.is_empty() {
            return text.to_string();
        }
        text.replace(&self.password, REDACTED)
    }
}

enum Call {
    Create,
    Delete,
}

pub struct FerriskeyAdmin {
    http: reqwest::Client,
}

impl Default for FerriskeyAdmin {
    fn default() -> Self {
        Self::new()
    }
}

impl FerriskeyAdmin {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { http }
    }

    pub async fn create_realm(
        &self,
        base_url: &str,
        credentials: &AdminCredentials,
        realm: &str,
    ) -> Result<(), RealmError> {
        let token = self.token(base_url, credentials).await?;
        let response = self
            .http
            .post(format!("{base_url}/realms"))
            .bearer_auth(token)
            .header(CONTENT_TYPE, "application/json")
            .body(json!({ "name": realm }).to_string())
            .send()
            .await
            .map_err(unreachable)?;

        answer(Call::Create, response, credentials).await
    }

    pub async fn delete_realm(
        &self,
        base_url: &str,
        credentials: &AdminCredentials,
        realm: &str,
    ) -> Result<(), RealmError> {
        let token = self.token(base_url, credentials).await?;
        let response = self
            .http
            .delete(format!("{base_url}/realms/{realm}"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(unreachable)?;

        answer(Call::Delete, response, credentials).await
    }

    async fn token(
        &self,
        base_url: &str,
        credentials: &AdminCredentials,
    ) -> Result<String, RealmError> {
        let response = self
            .http
            .post(format!(
                "{base_url}/realms/{ADMIN_REALM}/protocol/openid-connect/token"
            ))
            .form(&[
                ("grant_type", "password"),
                ("client_id", ADMIN_CLIENT),
                ("username", credentials.username.as_str()),
                ("password", credentials.password.as_str()),
            ])
            .send()
            .await
            .map_err(unreachable)?;

        let status = response.status();
        if !status.is_success() {
            return Err(RealmError::CellUnreachable {
                message: format!("FerrisKey refused an admin token (status {status})"),
            });
        }

        let body = response.text().await.unwrap_or_default();
        serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| value["access_token"].as_str().map(ToOwned::to_owned))
            .ok_or_else(|| RealmError::CellUnreachable {
                message: "FerrisKey answered the token request without a token".to_string(),
            })
    }
}

async fn answer(
    call: Call,
    response: reqwest::Response,
    credentials: &AdminCredentials,
) -> Result<(), RealmError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    match (call, status) {
        (Call::Create, StatusCode::CONFLICT) => return Err(RealmError::AlreadyExists),
        (Call::Delete, StatusCode::NOT_FOUND) => return Err(RealmError::NotFound),
        _ => {}
    }

    let message = credentials.redact(&error_message(
        status.as_u16(),
        &response.text().await.unwrap_or_default(),
    ));
    if status.is_server_error() || matches!(status.as_u16(), 401 | 408 | 429) {
        Err(RealmError::CellUnreachable { message })
    } else {
        Err(RealmError::Refused { reason: message })
    }
}

fn unreachable(error: reqwest::Error) -> RealmError {
    let state = if error.is_timeout() {
        "timed out"
    } else {
        "unreachable"
    };
    RealmError::CellUnreachable {
        message: format!("FerrisKey is {state}"),
    }
}

fn error_message(status: u16, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["message"].as_str().map(ToOwned::to_owned))
        .map(|message| format!("FerrisKey answered {status}: {message}"))
        .unwrap_or_else(|| format!("FerrisKey answered {status}"))
}

fn api_base_url(cell: &IdentityInstanceRef) -> String {
    format!(
        "http://{}-api.{}.svc:{API_PORT}{API_ROOT_PATH}",
        cell.name, cell.namespace
    )
}

fn admin_secret_name(cell: &IdentityInstanceRef) -> String {
    format!("{}-api-admin", cell.name)
}

pub struct FerriskeyRealms {
    client: Client,
    admin: FerriskeyAdmin,
}

impl FerriskeyRealms {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            admin: FerriskeyAdmin::new(),
        }
    }

    async fn credentials(
        &self,
        cell: &IdentityInstanceRef,
    ) -> Result<AdminCredentials, RealmError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &cell.namespace);
        let secret = secrets
            .get_opt(&admin_secret_name(cell))
            .await
            .map_err(|_| RealmError::CellUnreachable {
                message: "the cell admin credentials could not be read".to_string(),
            })?;

        let read = |key: &str| {
            secret
                .as_ref()
                .and_then(|secret| secret.data.as_ref())
                .and_then(|data| data.get(key))
                .and_then(|value| String::from_utf8(value.0.clone()).ok())
        };

        match (read("username"), read("password")) {
            (Some(username), Some(password)) => Ok(AdminCredentials::new(username, password)),
            _ => Err(RealmError::CellUnreachable {
                message: "the cell admin credentials are not there yet".to_string(),
            }),
        }
    }
}

impl RealmPort for FerriskeyRealms {
    async fn create_realm(
        &self,
        cell: &IdentityInstanceRef,
        realm: &str,
    ) -> Result<(), RealmError> {
        let credentials = self.credentials(cell).await?;
        self.admin
            .create_realm(&api_base_url(cell), &credentials, realm)
            .await
    }

    async fn delete_realm(
        &self,
        cell: &IdentityInstanceRef,
        realm: &str,
    ) -> Result<(), RealmError> {
        let credentials = self.credentials(cell).await?;
        self.admin
            .delete_realm(&api_base_url(cell), &credentials, realm)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use uuid::Uuid;

    use super::*;

    struct Recorded {
        head: String,
        body: String,
    }

    fn serve(script: Vec<(u16, &'static str)>) -> (String, JoinHandle<Vec<Recorded>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bound");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let handle = thread::spawn(move || {
            script
                .into_iter()
                .map(|(status, body)| {
                    let (mut stream, _) = listener.accept().expect("accepted");
                    let recorded = read_request(&mut stream);
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).expect("written");
                    recorded
                })
                .collect()
        });
        (url, handle)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Recorded {
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        let split = loop {
            let read = stream.read(&mut buf).expect("read");
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
                    .map(|value| value.trim().parse::<usize>().expect("a length"))
            })
            .unwrap_or(0);
        while raw.len() < split + length {
            let read = stream.read(&mut buf).expect("read");
            raw.extend_from_slice(&buf[..read]);
        }
        Recorded {
            head,
            body: String::from_utf8_lossy(&raw[split..split + length]).to_string(),
        }
    }

    const TOKEN_OK: (u16, &str) = (200, r#"{"access_token":"fake-token"}"#);
    const PASSWORD: &str = "s3cret-cell-pass";

    fn credentials() -> AdminCredentials {
        AdminCredentials::new("admin", PASSWORD)
    }

    #[tokio::test]
    async fn a_realm_is_created_with_a_master_admin_token() {
        let (url, server) = serve(vec![TOKEN_OK, (201, "{}")]);

        FerriskeyAdmin::new()
            .create_realm(&url, &credentials(), "acme")
            .await
            .expect("created");

        let seen = server.join().expect("served");
        assert!(
            seen[0]
                .head
                .starts_with("POST /realms/master/protocol/openid-connect/token ")
        );
        assert!(seen[0].body.contains("client_id=admin-cli"));
        assert!(seen[1].head.starts_with("POST /realms "));
        assert!(
            seen[1]
                .head
                .to_ascii_lowercase()
                .contains("authorization: bearer fake-token")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&seen[1].body).expect("json"),
            json!({ "name": "acme" })
        );
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn a_taken_name_is_an_existing_realm() {
        let (url, server) = serve(vec![TOKEN_OK, (409, r#"{"message":"exists"}"#)]);

        let result = FerriskeyAdmin::new()
            .create_realm(&url, &credentials(), "acme")
            .await;

        assert_eq!(result, Err(RealmError::AlreadyExists));
        server.join().expect("served");
    }

    /// @spec-fpr-8
    #[tokio::test]
    async fn deleting_an_absent_realm_is_not_found() {
        let (url, server) = serve(vec![TOKEN_OK, (404, r#"{"message":"no realm"}"#)]);

        let result = FerriskeyAdmin::new()
            .delete_realm(&url, &credentials(), "acme")
            .await;

        assert_eq!(result, Err(RealmError::NotFound));
        let seen = server.join().expect("served");
        assert!(seen[1].head.starts_with("DELETE /realms/acme "));
    }

    /// @spec-fpr-7
    #[tokio::test]
    async fn a_client_error_is_a_refusal_with_the_message_from_ferriskey() {
        let (url, server) = serve(vec![TOKEN_OK, (400, r#"{"message":"invalid name"}"#)]);

        let result = FerriskeyAdmin::new()
            .create_realm(&url, &credentials(), "acme")
            .await;

        assert_eq!(
            result,
            Err(RealmError::Refused {
                reason: "FerrisKey answered 400: invalid name".to_string()
            })
        );
        server.join().expect("served");
    }

    #[tokio::test]
    async fn a_server_error_or_a_refused_token_is_retried() {
        for script in [
            vec![TOKEN_OK, (503, "{}")],
            vec![(401, r#"{"message":"bad credentials"}"#)],
        ] {
            let (url, server) = serve(script);

            let result = FerriskeyAdmin::new()
                .create_realm(&url, &credentials(), "acme")
                .await;

            assert!(matches!(result, Err(RealmError::CellUnreachable { .. })));
            server.join().expect("served");
        }
    }

    #[tokio::test]
    async fn an_unreachable_cell_is_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bound");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        drop(listener);

        let result = FerriskeyAdmin::new()
            .delete_realm(&url, &credentials(), "acme")
            .await;

        assert!(matches!(result, Err(RealmError::CellUnreachable { .. })));
    }

    /// @spec-fpr-14
    #[tokio::test]
    async fn the_cell_password_never_reaches_an_error() {
        let echo: &'static str =
            Box::leak(format!(r#"{{"message":"bad password {PASSWORD}"}}"#).into_boxed_str());
        for script in [
            vec![TOKEN_OK, (400, echo)],
            vec![TOKEN_OK, (500, echo)],
            vec![(401, echo)],
        ] {
            let (url, server) = serve(script);

            let error = FerriskeyAdmin::new()
                .create_realm(&url, &credentials(), "acme")
                .await
                .expect_err("failed");

            assert!(
                !format!("{error} {error:?}").contains(PASSWORD),
                "{error:?}"
            );
            server.join().expect("served");
        }
    }

    #[test]
    fn the_cell_is_reached_through_its_api_service_with_its_admin_secret() {
        let cell = IdentityInstanceRef::for_deployment(Uuid::from_u128(2), "cells-one");

        assert_eq!(
            api_base_url(&cell),
            format!("http://{}-api.cells-one.svc:3333/api", cell.name)
        );
        assert_eq!(admin_secret_name(&cell), format!("{}-api-admin", cell.name));
    }
}
