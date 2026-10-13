use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::{IdentityInstance, IdentityProvider};
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use reqwest::{RequestBuilder, StatusCode, header::CONTENT_TYPE, redirect::Policy};
use serde_json::{Value, json};

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::realm_payload::RealmName;
use crate::domain::entities::tenant_access::{
    ADMINISTRATOR_USERNAME, AccessGrant, CONSOLE_CLIENT, InitialAdministrator, TENANT_PERMISSIONS,
    TENANT_ROLE, TENANT_ROLE_DESCRIPTION, TenantAccess,
};
use crate::domain::error::{GenesisError, RealmError};
use crate::domain::ports::RealmPort;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const ADMIN_REALM: &str = "master";
const ADMIN_CLIENT: &str = "admin-cli";
const API_PORT: u16 = 3333;
const API_ROOT_PATH: &str = "/api";

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
}

enum Call {
    Create,
    Delete,
}

pub struct FerriskeyAdmin {
    http: reqwest::Client,
}

impl FerriskeyAdmin {
    pub fn new() -> Result<Self, GenesisError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .build()
            .map_err(|error| GenesisError::Internal {
                message: format!("the FerrisKey admin client could not be built: {error}"),
            })?;
        Ok(Self { http })
    }

    pub async fn create_realm(
        &self,
        base_url: &str,
        credentials: &AdminCredentials,
        realm: &RealmName,
    ) -> Result<(), RealmError> {
        let token = self.token(base_url, credentials).await?;
        let response = self
            .http
            .post(format!("{base_url}/realms"))
            .bearer_auth(token)
            .header(CONTENT_TYPE, "application/json")
            .body(json!({ "name": realm.as_str() }).to_string())
            .send()
            .await
            .map_err(unreachable)?;

        answer(Call::Create, response.status())
    }

    pub async fn delete_realm(
        &self,
        base_url: &str,
        credentials: &AdminCredentials,
        realm: &RealmName,
    ) -> Result<(), RealmError> {
        let token = self.token(base_url, credentials).await?;
        let response = self
            .http
            .delete(format!("{base_url}/realms/{realm}"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(unreachable)?;

        answer(Call::Delete, response.status())
    }

    pub async fn open_access(
        &self,
        base_url: &str,
        credentials: &AdminCredentials,
        access: &TenantAccess,
    ) -> Result<AccessGrant, RealmError> {
        let token = self.token(base_url, credentials).await?;
        let realm = format!("{base_url}/realms/{}", access.realm);
        let user = format!("{realm}/users/{}", access.administrator_id);

        let client_id = self.console_client(&realm, &token).await?;
        self.ensure_redirect(&realm, &client_id, &access.console_callback, &token)
            .await?;
        let role_id = self.ensure_role(&realm, &token).await?;
        self.ensure_administrator(&realm, &user, access, &token)
            .await?;
        self.ensure_assignment(&user, &role_id, &token).await?;
        self.issue_password(&user, &token).await
    }

    async fn exchange(
        &self,
        request: RequestBuilder,
        token: &str,
        step: &'static str,
    ) -> Result<(StatusCode, Value), RealmError> {
        let response = request
            .bearer_auth(token)
            .send()
            .await
            .map_err(unreachable)?;
        let status = response.status();
        let body = response.text().await.map_err(unreachable)?;
        if !status.is_success() || body.trim().is_empty() {
            return Ok((status, Value::Null));
        }
        let value = serde_json::from_str(&body).map_err(|_| RealmError::CellUnreachable {
            message: format!("{step} answered a body that is not JSON"),
        })?;
        Ok((status, value))
    }

    async fn read(
        &self,
        url: String,
        token: &str,
        step: &'static str,
    ) -> Result<Value, RealmError> {
        let (status, value) = self.exchange(self.http.get(url), token, step).await?;
        succeeded(step, status)?;
        Ok(value)
    }

    async fn write(
        &self,
        request: RequestBuilder,
        body: Value,
        token: &str,
        step: &'static str,
    ) -> Result<Value, RealmError> {
        let request = request
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string());
        let (status, value) = self.exchange(request, token, step).await?;
        succeeded(step, status)?;
        Ok(value)
    }

    async fn console_client(&self, realm: &str, token: &str) -> Result<String, RealmError> {
        let found = self
            .read(
                format!("{realm}/clients?client_id_exact={CONSOLE_CLIENT}"),
                token,
                STEP_CLIENTS,
            )
            .await?;
        items(&found)
            .iter()
            .find(|client| client["client_id"].as_str() == Some(CONSOLE_CLIENT))
            .and_then(|client| client["id"].as_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| RealmError::CellUnreachable {
                message: "the console client is not in the realm yet".to_string(),
            })
    }

    async fn ensure_redirect(
        &self,
        realm: &str,
        client_id: &str,
        callback: &str,
        token: &str,
    ) -> Result<(), RealmError> {
        let url = format!("{realm}/clients/{client_id}/redirects");
        let known = self.read(url.clone(), token, STEP_REDIRECTS).await?;
        if items(&known)
            .iter()
            .any(|redirect| redirect["value"].as_str() == Some(callback))
        {
            return Ok(());
        }
        self.write(
            self.http.post(url),
            json!({ "value": callback, "enabled": true }),
            token,
            STEP_REDIRECT_ADD,
        )
        .await
        .map(drop)
    }

    async fn ensure_role(&self, realm: &str, token: &str) -> Result<String, RealmError> {
        let found = self
            .read(
                format!("{realm}/roles?name={TENANT_ROLE}"),
                token,
                STEP_ROLES,
            )
            .await?;
        let existing = items(&found)
            .iter()
            .find(|role| role["name"].as_str() == Some(TENANT_ROLE) && role["client_id"].is_null());
        let id = match existing {
            Some(role) => role["id"].as_str().map(ToOwned::to_owned),
            None => {
                let created = self
                    .write(
                        self.http.post(format!("{realm}/roles")),
                        json!({
                            "name": TENANT_ROLE,
                            "description": TENANT_ROLE_DESCRIPTION,
                            "permissions": TENANT_PERMISSIONS,
                        }),
                        token,
                        STEP_ROLE_ADD,
                    )
                    .await?;
                created["data"]["id"].as_str().map(ToOwned::to_owned)
            }
        };
        id.ok_or_else(|| RealmError::CellUnreachable {
            message: format!("{STEP_ROLES} answered a role without an id"),
        })
    }

    async fn ensure_administrator(
        &self,
        realm: &str,
        user: &str,
        access: &TenantAccess,
        token: &str,
    ) -> Result<(), RealmError> {
        let (status, _) = self.exchange(self.http.get(user), token, STEP_USER).await?;
        if status.is_success() {
            return Ok(());
        }
        if status != StatusCode::NOT_FOUND {
            return Err(fault(STEP_USER, status));
        }

        let request = self
            .http
            .post(format!("{realm}/users"))
            .header(CONTENT_TYPE, "application/json")
            .body(
                json!({ "id": access.administrator_id, "username": ADMINISTRATOR_USERNAME })
                    .to_string(),
            );
        let (status, _) = self.exchange(request, token, STEP_USER_ADD).await?;
        match status {
            status if status.is_success() => Ok(()),
            StatusCode::BAD_REQUEST => Err(RealmError::CellUnreachable {
                message: format!("{STEP_USER_ADD} raced another attempt"),
            }),
            status => Err(fault(STEP_USER_ADD, status)),
        }
    }

    async fn ensure_assignment(
        &self,
        user: &str,
        role_id: &str,
        token: &str,
    ) -> Result<(), RealmError> {
        let url = format!("{user}/roles");
        let assigned = self.read(url.clone(), token, STEP_ASSIGNMENTS).await?;
        if items(&assigned)
            .iter()
            .any(|role| role["id"].as_str() == Some(role_id))
        {
            return Ok(());
        }
        let (status, _) = self
            .exchange(
                self.http.post(format!("{url}/{role_id}")),
                token,
                STEP_ASSIGN,
            )
            .await?;
        succeeded(STEP_ASSIGN, status)
    }

    async fn issue_password(&self, user: &str, token: &str) -> Result<AccessGrant, RealmError> {
        let credentials = self
            .read(format!("{user}/credentials"), token, STEP_CREDENTIALS)
            .await?;
        if items(&credentials)
            .iter()
            .any(|credential| credential["credential_type"].as_str() == Some("password"))
        {
            return Ok(AccessGrant::AlreadyIssued);
        }

        let administrator = InitialAdministrator::generate(ADMINISTRATOR_USERNAME);
        self.write(
            self.http.put(format!("{user}/reset-password")),
            json!({
                "temporary": true,
                "credential_type": "password",
                "value": administrator.password(),
            }),
            token,
            STEP_PASSWORD,
        )
        .await?;
        Ok(AccessGrant::Issued(administrator))
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
                message: format!("admin token refused by the cell ({})", status.as_u16()),
            });
        }

        let body = response.text().await.unwrap_or_default();
        serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| value["access_token"].as_str().map(ToOwned::to_owned))
            .ok_or_else(|| RealmError::CellUnreachable {
                message: "the cell answered the token request without a token".to_string(),
            })
    }
}

fn answer(call: Call, status: StatusCode) -> Result<(), RealmError> {
    if status.is_success() {
        return Ok(());
    }

    let code = status.as_u16();
    match (call, status) {
        (Call::Create, StatusCode::CONFLICT) => Err(RealmError::AlreadyExists),
        (Call::Delete, StatusCode::NOT_FOUND) => Err(RealmError::NotFound),
        (_, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY) => {
            Err(RealmError::Refused {
                reason: format!("realm refused by the cell ({code})"),
            })
        }
        _ => Err(RealmError::CellUnreachable {
            message: format!("the cell answered {code}"),
        }),
    }
}

const STEP_CLIENTS: &str = "the console client lookup";
const STEP_REDIRECTS: &str = "the redirect lookup";
const STEP_REDIRECT_ADD: &str = "the redirect registration";
const STEP_ROLES: &str = "the role lookup";
const STEP_ROLE_ADD: &str = "the role creation";
const STEP_USER: &str = "the administrator lookup";
const STEP_USER_ADD: &str = "the administrator creation";
const STEP_ASSIGNMENTS: &str = "the role assignment lookup";
const STEP_ASSIGN: &str = "the role assignment";
const STEP_CREDENTIALS: &str = "the credential lookup";
const STEP_PASSWORD: &str = "the password reset";

fn items(value: &Value) -> &[Value] {
    value
        .as_array()
        .or_else(|| value["data"].as_array())
        .map_or(&[], Vec::as_slice)
}

fn fault(step: &str, status: StatusCode) -> RealmError {
    RealmError::CellUnreachable {
        message: format!("{step} answered {}", status.as_u16()),
    }
}

fn succeeded(step: &str, status: StatusCode) -> Result<(), RealmError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(fault(step, status))
    }
}

fn unreachable(error: reqwest::Error) -> RealmError {
    let state = if error.is_timeout() {
        "timed out"
    } else {
        "unreachable"
    };
    RealmError::CellUnreachable {
        message: format!("the cell is {state}"),
    }
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

struct FoundInstance<'a> {
    name: Option<&'a str>,
    namespace: Option<&'a str>,
    provider: &'a IdentityProvider,
    pooled_cell: bool,
}

impl<'a> From<&'a IdentityInstance> for FoundInstance<'a> {
    fn from(instance: &'a IdentityInstance) -> Self {
        Self {
            name: instance.metadata.name.as_deref(),
            namespace: instance.metadata.namespace.as_deref(),
            provider: &instance.spec.provider,
            pooled_cell: instance
                .spec
                .ferriskey
                .as_ref()
                .is_some_and(|config| config.pooled_cell),
        }
    }
}

fn is_ferriskey_cell(found: FoundInstance<'_>, cell: &IdentityInstanceRef) -> bool {
    found.name == Some(cell.name.as_str())
        && found.namespace == Some(cell.namespace.as_str())
        && *found.provider == IdentityProvider::Ferriskey
        && found.pooled_cell
}

pub struct FerriskeyRealms {
    client: Client,
    admin: FerriskeyAdmin,
}

impl FerriskeyRealms {
    pub fn new(client: Client) -> Result<Self, GenesisError> {
        Ok(Self {
            client,
            admin: FerriskeyAdmin::new()?,
        })
    }

    async fn ensure_cell(&self, cell: &IdentityInstanceRef) -> Result<(), RealmError> {
        let instances: Api<IdentityInstance> =
            Api::namespaced(self.client.clone(), &cell.namespace);
        let instance =
            instances
                .get_opt(&cell.name)
                .await
                .map_err(|_| RealmError::CellUnreachable {
                    message: "the cell instance could not be read".to_string(),
                })?;

        match instance {
            Some(instance) if is_ferriskey_cell(FoundInstance::from(&instance), cell) => Ok(()),
            _ => Err(RealmError::NotACell),
        }
    }

    async fn credentials(
        &self,
        cell: &IdentityInstanceRef,
    ) -> Result<AdminCredentials, RealmError> {
        self.ensure_cell(cell).await?;

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
        realm: &RealmName,
    ) -> Result<(), RealmError> {
        let credentials = self.credentials(cell).await?;
        self.admin
            .create_realm(&api_base_url(cell), &credentials, realm)
            .await
    }

    async fn delete_realm(
        &self,
        cell: &IdentityInstanceRef,
        realm: &RealmName,
    ) -> Result<(), RealmError> {
        let credentials = self.credentials(cell).await?;
        self.admin
            .delete_realm(&api_base_url(cell), &credentials, realm)
            .await
    }

    async fn open_access(
        &self,
        cell: &IdentityInstanceRef,
        access: &TenantAccess,
    ) -> Result<AccessGrant, RealmError> {
        let credentials = self.credentials(cell).await?;
        self.admin
            .open_access(&api_base_url(cell), &credentials, access)
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
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nLocation: /elsewhere\r\nConnection: close\r\n\r\n{body}",
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

    fn acme() -> RealmName {
        RealmName::try_from("acme").expect("valid")
    }

    #[tokio::test]
    async fn a_realm_is_created_with_a_master_admin_token() {
        let (url, server) = serve(vec![TOKEN_OK, (201, "{}")]);

        FerriskeyAdmin::new()
            .expect("built")
            .create_realm(&url, &credentials(), &acme())
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
            .expect("built")
            .create_realm(&url, &credentials(), &acme())
            .await;

        assert_eq!(result, Err(RealmError::AlreadyExists));
        server.join().expect("served");
    }

    /// @spec-fpr-8
    #[tokio::test]
    async fn deleting_an_absent_realm_is_not_found() {
        let (url, server) = serve(vec![TOKEN_OK, (404, r#"{"message":"no realm"}"#)]);

        let result = FerriskeyAdmin::new()
            .expect("built")
            .delete_realm(&url, &credentials(), &acme())
            .await;

        assert_eq!(result, Err(RealmError::NotFound));
        let seen = server.join().expect("served");
        assert!(seen[1].head.starts_with("DELETE /realms/acme "));
    }

    /// @spec-fpr-7
    #[tokio::test]
    async fn only_a_bad_request_or_an_unprocessable_name_is_a_refusal() {
        for status in [400, 422] {
            let (url, server) = serve(vec![TOKEN_OK, (status, r#"{"message":"invalid name"}"#)]);

            let result = FerriskeyAdmin::new()
                .expect("built")
                .create_realm(&url, &credentials(), &acme())
                .await;

            assert_eq!(
                result,
                Err(RealmError::Refused {
                    reason: format!("realm refused by the cell ({status})")
                })
            );
            server.join().expect("served");
        }
    }

    #[tokio::test]
    async fn any_other_answer_is_a_cell_fault_and_is_retried() {
        for status in [401, 403, 404, 405, 408, 415, 418, 429, 500, 503] {
            let (url, server) = serve(vec![TOKEN_OK, (status, "{}")]);

            let result = FerriskeyAdmin::new()
                .expect("built")
                .create_realm(&url, &credentials(), &acme())
                .await;

            assert_eq!(
                result,
                Err(RealmError::CellUnreachable {
                    message: format!("the cell answered {status}")
                })
            );
            server.join().expect("served");
        }
    }

    #[tokio::test]
    async fn a_redirect_is_never_followed() {
        let (url, server) = serve(vec![TOKEN_OK, (307, "{}")]);

        let result = FerriskeyAdmin::new()
            .expect("built")
            .delete_realm(&url, &credentials(), &acme())
            .await;

        assert_eq!(
            result,
            Err(RealmError::CellUnreachable {
                message: "the cell answered 307".to_string()
            })
        );
        assert_eq!(server.join().expect("served").len(), 2);
    }

    #[tokio::test]
    async fn a_refused_token_is_retried() {
        let (url, server) = serve(vec![(401, r#"{"message":"bad credentials"}"#)]);

        let result = FerriskeyAdmin::new()
            .expect("built")
            .create_realm(&url, &credentials(), &acme())
            .await;

        assert!(matches!(result, Err(RealmError::CellUnreachable { .. })));
        assert_eq!(server.join().expect("served").len(), 1);
    }

    #[tokio::test]
    async fn an_unreachable_cell_is_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bound");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        drop(listener);

        let result = FerriskeyAdmin::new()
            .expect("built")
            .delete_realm(&url, &credentials(), &acme())
            .await;

        assert!(matches!(result, Err(RealmError::CellUnreachable { .. })));
    }

    /// @spec-fpr-14
    #[tokio::test]
    async fn nothing_the_cell_answers_reaches_an_error() {
        let echo: &'static str = Box::leak(
            format!(r#"{{"message":"bad password {PASSWORD} from upstream"}}"#).into_boxed_str(),
        );
        for script in [
            vec![TOKEN_OK, (400, echo)],
            vec![TOKEN_OK, (500, echo)],
            vec![(401, echo)],
        ] {
            let (url, server) = serve(script);

            let error = FerriskeyAdmin::new()
                .expect("built")
                .create_realm(&url, &credentials(), &acme())
                .await
                .expect_err("failed");

            let shown = format!("{error} {error:?}");
            assert!(!shown.contains(PASSWORD), "{shown}");
            assert!(!shown.contains("upstream"), "{shown}");
            server.join().expect("served");
        }
    }

    const CLIENTS: (u16, &str) = (
        200,
        r#"{"data":[{"id":"other","client_id":"x"},{"id":"c1","client_id":"security-admin-console"}],"metadata":{"total":2}}"#,
    );
    const REDIRECTS_EMPTY: (u16, &str) = (200, "[]");
    const REDIRECTS_KNOWN: (u16, &str) = (
        200,
        r#"[{"id":"x","value":"https://acme.cells.example/realms/acme/authentication/callback","enabled":true}]"#,
    );
    const ROLES_EMPTY: (u16, &str) = (200, r#"{"data":[]}"#);
    const ROLES_KNOWN: (u16, &str) = (
        200,
        r#"{"data":[{"id":"decoy","name":"tenant-admin","client_id":"c9"},{"id":"r1","name":"tenant-admin","client_id":null}],"metadata":{}}"#,
    );
    const ROLE_CREATED: (u16, &str) = (201, r#"{"data":{"id":"r1","name":"tenant-admin"}}"#);
    const USER_KNOWN: (u16, &str) = (200, r#"{"data":{"username":"admin"}}"#);
    const USER_ABSENT: (u16, &str) = (404, r#"{"message":"no user"}"#);
    const ASSIGNED_NONE: (u16, &str) = (200, r#"{"data":[]}"#);
    const ASSIGNED_KNOWN: (u16, &str) = (200, r#"{"data":[{"id":"r1"}]}"#);
    const CREDENTIALS_NONE: (u16, &str) = (200, r#"{"data":[]}"#);
    const CREDENTIALS_SET: (u16, &str) = (200, r#"{"data":[{"credential_type":"password"}]}"#);
    const OK_EMPTY: (u16, &str) = (200, "{}");
    const CREATED_EMPTY: (u16, &str) = (201, "{}");

    fn tenant_access() -> TenantAccess {
        let payload = crate::domain::entities::realm_payload::RealmPayloadV1::from_value(&json!({
            "deployment_id": Uuid::from_u128(1),
            "cell": { "deployment_id": Uuid::from_u128(2), "namespace": "cells-one" },
            "realm": "acme",
            "hostname": "acme.cells.example",
        }))
        .expect("a payload");
        TenantAccess::for_realm(&payload, "acme.cells.example")
    }

    fn lines(seen: &[Recorded]) -> Vec<String> {
        seen.iter()
            .skip(1)
            .map(|recorded| {
                let mut words = recorded.head.split_whitespace();
                format!(
                    "{} {}",
                    words.next().unwrap_or_default(),
                    words.next().unwrap_or_default()
                )
            })
            .collect()
    }

    async fn open(
        script: Vec<(u16, &'static str)>,
    ) -> (Result<AccessGrant, RealmError>, Vec<Recorded>) {
        let (url, server) = serve(script);
        let result = FerriskeyAdmin::new()
            .expect("built")
            .open_access(&url, &credentials(), &tenant_access())
            .await;
        (result, server.join().expect("served"))
    }

    const USER: &str = "/api-less/realms/acme/users/00000000-0000-0000-0000-000000000001";

    fn user_path(suffix: &str) -> String {
        USER.replacen("/api-less", "", 1) + suffix
    }

    #[tokio::test]
    async fn the_first_run_opens_the_access_in_order_and_issues_a_temporary_password() {
        let (result, seen) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_EMPTY,
            CREATED_EMPTY,
            ROLES_EMPTY,
            ROLE_CREATED,
            USER_ABSENT,
            CREATED_EMPTY,
            ASSIGNED_NONE,
            OK_EMPTY,
            CREDENTIALS_NONE,
            OK_EMPTY,
        ])
        .await;

        let Ok(AccessGrant::Issued(administrator)) = result else {
            panic!("expected an issued credential");
        };
        assert_eq!(administrator.username(), "admin");
        assert_eq!(
            lines(&seen),
            [
                "GET /realms/acme/clients?client_id_exact=security-admin-console".to_string(),
                "GET /realms/acme/clients/c1/redirects".to_string(),
                "POST /realms/acme/clients/c1/redirects".to_string(),
                "GET /realms/acme/roles?name=tenant-admin".to_string(),
                "POST /realms/acme/roles".to_string(),
                format!("GET {}", user_path("")),
                "POST /realms/acme/users".to_string(),
                format!("GET {}", user_path("/roles")),
                format!("POST {}", user_path("/roles/r1")),
                format!("GET {}", user_path("/credentials")),
                format!("PUT {}", user_path("/reset-password")),
            ]
        );
        for recorded in &seen[1..] {
            assert!(
                recorded
                    .head
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fake-token")
            );
        }
        let json_of = |at: usize| serde_json::from_str::<Value>(&seen[at].body).expect("json");
        assert_eq!(
            json_of(3),
            json!({"value": "https://acme.cells.example/realms/acme/authentication/callback", "enabled": true})
        );
        assert_eq!(
            json_of(7),
            json!({"id": "00000000-0000-0000-0000-000000000001", "username": "admin"})
        );
        let reset = json_of(11);
        assert_eq!(reset["temporary"], json!(true));
        assert_eq!(reset["credential_type"], json!("password"));
        assert_eq!(reset["value"].as_str(), Some(administrator.password()));
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn a_run_where_everything_exists_writes_nothing_and_keeps_the_password() {
        let (result, seen) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_KNOWN,
            ROLES_KNOWN,
            USER_KNOWN,
            ASSIGNED_KNOWN,
            CREDENTIALS_SET,
        ])
        .await;

        assert!(matches!(result, Ok(AccessGrant::AlreadyIssued)));
        assert!(lines(&seen).iter().all(|line| line.starts_with("GET ")));
        assert_eq!(seen.len(), 7);
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn an_existing_redirect_is_not_added_twice() {
        let (result, seen) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_KNOWN,
            ROLES_EMPTY,
            ROLE_CREATED,
            USER_KNOWN,
            ASSIGNED_KNOWN,
            CREDENTIALS_SET,
        ])
        .await;

        assert!(result.is_ok());
        assert!(!lines(&seen).contains(&"POST /realms/acme/clients/c1/redirects".to_string()));
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn an_existing_role_is_reused_and_a_client_role_of_that_name_is_not() {
        let (result, seen) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_KNOWN,
            ROLES_KNOWN,
            USER_KNOWN,
            ASSIGNED_NONE,
            OK_EMPTY,
            CREDENTIALS_SET,
        ])
        .await;

        assert!(result.is_ok());
        let sent = lines(&seen);
        assert!(!sent.contains(&"POST /realms/acme/roles".to_string()));
        assert!(sent.contains(&format!("POST {}", user_path("/roles/r1"))));
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn an_existing_role_assignment_is_not_repeated() {
        let (result, seen) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_KNOWN,
            ROLES_KNOWN,
            USER_KNOWN,
            ASSIGNED_KNOWN,
            CREDENTIALS_SET,
        ])
        .await;

        assert!(result.is_ok());
        assert!(
            !lines(&seen)
                .iter()
                .any(|line| line.starts_with("POST ") && line.contains("/roles/"))
        );
    }

    #[tokio::test]
    async fn the_permissions_sent_never_include_manage_realm() {
        let (_, seen) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_KNOWN,
            ROLES_EMPTY,
            ROLE_CREATED,
            USER_KNOWN,
            ASSIGNED_KNOWN,
            CREDENTIALS_SET,
        ])
        .await;

        let body = serde_json::from_str::<Value>(&seen[4].body).expect("json");
        assert_eq!(body["name"], json!("tenant-admin"));
        let permissions = body["permissions"].as_array().expect("a list");
        assert_eq!(permissions.len(), 15);
        assert!(!permissions.contains(&json!("manage_realm")));
    }

    #[tokio::test]
    async fn a_user_created_concurrently_is_retried_not_failed() {
        let (result, _) = open(vec![
            TOKEN_OK,
            CLIENTS,
            REDIRECTS_KNOWN,
            ROLES_KNOWN,
            USER_ABSENT,
            (400, "{}"),
        ])
        .await;

        assert!(matches!(result, Err(RealmError::CellUnreachable { .. })));
    }

    /// @spec-fpr-14
    #[tokio::test]
    async fn a_failing_step_names_neither_the_password_nor_the_body() {
        let echo: &'static str =
            Box::leak(format!(r#"{{"message":"{PASSWORD} upstream"}}"#).into_boxed_str());
        for script in [
            vec![TOKEN_OK, (500, echo)],
            vec![
                TOKEN_OK,
                CLIENTS,
                REDIRECTS_KNOWN,
                ROLES_KNOWN,
                USER_KNOWN,
                ASSIGNED_KNOWN,
                CREDENTIALS_NONE,
                (500, echo),
            ],
            vec![
                TOKEN_OK,
                CLIENTS,
                REDIRECTS_KNOWN,
                ROLES_KNOWN,
                USER_KNOWN,
                (200, "not json upstream"),
            ],
        ] {
            let (result, _) = open(script).await;

            let Err(error) = result else {
                panic!("expected a failure");
            };
            let shown = format!("{error} {error:?}");
            assert!(!shown.contains(PASSWORD), "{shown}");
            assert!(!shown.contains("upstream"), "{shown}");
        }
    }

    #[test]
    fn only_a_ferriskey_instance_with_the_payload_name_and_namespace_is_a_cell() {
        let cell = IdentityInstanceRef::for_deployment(Uuid::from_u128(2), "cells-one");
        let found = |name, namespace, provider| FoundInstance {
            name: Some(name),
            namespace: Some(namespace),
            provider,
            pooled_cell: true,
        };
        let name = cell.name.as_str();

        assert!(is_ferriskey_cell(
            found(name, "cells-one", &IdentityProvider::Ferriskey),
            &cell
        ));
        assert!(!is_ferriskey_cell(
            found(name, "cells-one", &IdentityProvider::Keycloak),
            &cell
        ));
        assert!(!is_ferriskey_cell(
            found(name, "other", &IdentityProvider::Ferriskey),
            &cell
        ));
        assert!(!is_ferriskey_cell(
            found("deployment-x", "cells-one", &IdentityProvider::Ferriskey),
            &cell
        ));
    }

    #[test]
    fn a_ferriskey_instance_without_the_pooled_cell_marker_is_not_a_cell() {
        let cell = IdentityInstanceRef::for_deployment(Uuid::from_u128(2), "cells-one");
        let dedicated = FoundInstance {
            name: Some(cell.name.as_str()),
            namespace: Some("cells-one"),
            provider: &IdentityProvider::Ferriskey,
            pooled_cell: false,
        };

        assert!(!is_ferriskey_cell(dedicated, &cell));
    }

    #[test]
    fn the_pooled_cell_marker_is_read_from_the_ferriskey_spec() {
        let mut instance: IdentityInstance = serde_json::from_value(serde_json::json!({
            "apiVersion": "autharie.fr/v1alpha",
            "kind": "IdentityInstance",
            "metadata": { "name": "deployment-2", "namespace": "cells-one" },
            "spec": {
                "organisationId": "org-1",
                "provider": "ferriskey",
                "version": "26.0.1",
                "hostname": "cell.autharie.test",
                "ferriskey": { "pooledCell": true },
                "database": {
                    "mode": "managedCluster",
                    "managedCluster": {
                        "instances": 2,
                        "storage": { "size": "1Gi" },
                        "resources": { "requests": { "cpu": "500m", "memory": "1Gi" } }
                    }
                }
            }
        }))
        .expect("an instance");
        assert!(FoundInstance::from(&instance).pooled_cell);

        instance.spec.ferriskey = None;
        assert!(!FoundInstance::from(&instance).pooled_cell);
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
