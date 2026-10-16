use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::{IdentityInstance, IdentityProvider};
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use reqwest::{StatusCode, header::CONTENT_TYPE, redirect::Policy};
use serde_json::{Value, json};

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::realm_payload::RealmName;
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
