use reqwest::Method;
use serde_json::{Value, json};

use crate::domain::agents::AgentsError;
use crate::domain::iam::result::AdminAccess;

use super::ferriskey_agents::{Caller, client_path, id_of, items, malformed, segment, send, text};
use super::ferriskey_webhooks::{ApiRequest, PAGE_LIMIT};

pub const ADMIN_REALM: &str = "master";
pub const ADMIN_CLIENT_ID: &str = "autharie-admin";
const ADMIN_CLIENT_NAME: &str = "Autharie admin access";
const ADMIN_ROLE: &str = "autharie-admin";
const ADMIN_PERMISSIONS: [&str; 1] = ["manage_realm"];

fn service_account_name() -> String {
    format!("service-account-{ADMIN_CLIENT_ID}")
}

fn find_client_request(realm: &str) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::GET, format!("/realms/{}/clients", segment(realm)?))
            .query("client_id_exact", ADMIN_CLIENT_ID)
            .query("limit", 10),
    )
}

fn create_client_request(realm: &str) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::POST, format!("/realms/{}/clients", segment(realm)?)).body(json!({
            "name": ADMIN_CLIENT_NAME,
            "client_id": ADMIN_CLIENT_ID,
            "client_type": "confidential",
            "service_account_enabled": true,
            "public_client": false,
            "enabled": true,
            "direct_access_grants_enabled": false,
            "token_exchange_enabled": false,
            "require_pkce": false,
            "redirect_uris": [],
        })),
    )
}

fn find_role_request(realm: &str) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::GET, format!("/realms/{}/roles", segment(realm)?))
            .query("qualified_name", ADMIN_ROLE)
            .query("scope", "realm")
            .query("limit", PAGE_LIMIT),
    )
}

fn create_role_request(realm: &str) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::POST, format!("/realms/{}/roles", segment(realm)?)).body(json!({
            "name": ADMIN_ROLE,
            "description": "Lets the deployment's customer administer this FerrisKey",
            "permissions": ADMIN_PERMISSIONS,
        })),
    )
}

fn find_service_account_request(realm: &str) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::GET, format!("/realms/{}/users", segment(realm)?))
            .query("username", service_account_name())
            .query("service_account", true)
            .query("limit", PAGE_LIMIT),
    )
}

fn assign_role_request(realm: &str, user: &str, role: &str) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::POST,
        format!(
            "/realms/{}/users/{}/roles/{}",
            segment(realm)?,
            segment(user)?,
            segment(role)?
        ),
    ))
}

fn secret_request(realm: &str, client: &str) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::GET,
        format!("{}/client-secret", client_path(realm, client)?),
    ))
}

async fn remove_previous_client<C: Caller>(caller: &C, realm: &str) -> Result<(), AgentsError> {
    let body = send(caller, find_client_request(realm)?).await?;
    let existing = items(&body)
        .into_iter()
        .find(|client| text(client, "client_id") == Some(ADMIN_CLIENT_ID));

    match existing {
        Some(client) => {
            let id = id_of(client, "a client")?;
            send(
                caller,
                ApiRequest::new(Method::DELETE, client_path(realm, &id)?),
            )
            .await
            .map(|_| ())
        }
        None => Ok(()),
    }
}

async fn ensure_role<C: Caller>(caller: &C, realm: &str) -> Result<String, AgentsError> {
    let body = send(caller, find_role_request(realm)?).await?;
    let existing = items(&body)
        .into_iter()
        .find(|role| text(role, "name") == Some(ADMIN_ROLE));

    match existing {
        Some(role) => id_of(role, "a role"),
        None => id_of(&send(caller, create_role_request(realm)?).await?, "a role"),
    }
}

async fn service_account<C: Caller>(caller: &C, realm: &str) -> Result<String, AgentsError> {
    let username = service_account_name();
    let body = send(caller, find_service_account_request(realm)?).await?;

    items(&body)
        .into_iter()
        .find(|user| text(user, "username") == Some(username.as_str()))
        .ok_or_else(|| malformed("a client without a service account"))
        .and_then(|user| id_of(user, "a user"))
}

fn secret_in(body: &Value) -> Option<&str> {
    text(body, "client_secret")
        .or_else(|| text(&body["data"], "client_secret"))
        .filter(|secret| !secret.is_empty())
}

pub(super) async fn issue<C: Caller>(caller: &C, realm: &str) -> Result<AdminAccess, AgentsError> {
    if realm != ADMIN_REALM {
        return Err(AgentsError::Rejected {
            message: format!("admin access is issued in the realm '{ADMIN_REALM}' only"),
        });
    }

    remove_previous_client(caller, realm).await?;
    let created = send(caller, create_client_request(realm)?).await?;
    let client = id_of(&created, "a client")?;
    let role = ensure_role(caller, realm).await?;
    let user = service_account(caller, realm).await?;
    send(caller, assign_role_request(realm, &user, &role)?).await?;
    let body = send(caller, secret_request(realm, &client)?).await?;
    let secret = secret_in(&body).ok_or_else(|| malformed("a client without a secret"))?;

    Ok(AdminAccess {
        client_id: ADMIN_CLIENT_ID.to_string(),
        client_secret: secret.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::ferriskey_webhooks::ApiFailure;
    use super::*;

    const SECRET: &str = "s3cr3t-do-not-leak-0123456789";
    const OLD: &str = "00000000-0000-4000-8000-0000000000a1";
    const NEW: &str = "00000000-0000-4000-8000-0000000000a2";
    const ROLE: &str = "00000000-0000-4000-8000-0000000000b1";
    const USER: &str = "00000000-0000-4000-8000-0000000000c1";

    type Logged = (Method, String, Option<Value>);

    struct World {
        log: Mutex<Vec<Logged>>,
        previous_client: bool,
        role: Option<Value>,
        fail_on: Option<(Method, &'static str)>,
    }

    impl World {
        fn new() -> Self {
            Self {
                log: Mutex::new(Vec::new()),
                previous_client: false,
                role: None,
                fail_on: None,
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.log
                .lock()
                .expect("not poisoned")
                .iter()
                .map(|(method, path, _)| (method.to_string(), path.clone()))
                .collect()
        }

        fn body_of(&self, method: Method, path: &str) -> Option<Value> {
            self.log
                .lock()
                .expect("not poisoned")
                .iter()
                .find(|(m, p, _)| *m == method && p == path)
                .and_then(|(_, _, body)| body.clone())
        }
    }

    impl Caller for World {
        async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
            self.log.lock().expect("not poisoned").push((
                request.method.clone(),
                request.path.clone(),
                request.body.clone(),
            ));
            if let Some((method, suffix)) = &self.fail_on
                && request.method == *method
                && request.path.ends_with(suffix)
            {
                return Err(ApiFailure::Retry("FerrisKey is unreachable".to_string()));
            }
            let path = request.path.as_str();
            Ok(match (request.method.as_str(), path) {
                ("GET", "/realms/master/clients") if self.previous_client => {
                    json!({ "data": [{ "id": OLD, "client_id": ADMIN_CLIENT_ID }] })
                }
                ("GET", "/realms/master/clients") => json!({ "data": [] }),
                ("DELETE", _) => json!({}),
                ("POST", "/realms/master/clients") => json!({ "data": { "id": NEW } }),
                ("GET", "/realms/master/roles") => match &self.role {
                    Some(role) => json!({ "data": [role] }),
                    None => json!({ "data": [] }),
                },
                ("POST", "/realms/master/roles") => json!({ "data": { "id": ROLE } }),
                ("GET", "/realms/master/users") => json!({ "data": [
                    { "id": "other", "username": "service-account-autharie-admin-2" },
                    { "id": USER, "username": "service-account-autharie-admin" }
                ] }),
                ("POST", _) => json!({}),
                ("GET", _) => json!({ "data": { "client_secret": SECRET } }),
                _ => json!({}),
            })
        }
    }

    fn step(method: &str, path: &str) -> (String, String) {
        (method.to_string(), path.to_string())
    }

    #[tokio::test]
    async fn a_first_issue_creates_the_client_the_role_and_the_assignment_in_order() {
        let world = World::new();

        let access = issue(&world, "master").await.expect("issued");

        assert_eq!(access.client_id, "autharie-admin");
        assert_eq!(access.client_secret, SECRET);
        assert_eq!(
            world.calls(),
            vec![
                step("GET", "/realms/master/clients"),
                step("POST", "/realms/master/clients"),
                step("GET", "/realms/master/roles"),
                step("POST", "/realms/master/roles"),
                step("GET", "/realms/master/users"),
                step("POST", &format!("/realms/master/users/{USER}/roles/{ROLE}")),
                step(
                    "GET",
                    &format!("/realms/master/clients/{NEW}/client-secret")
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_previous_client_is_deleted_before_the_new_one_is_created() {
        let mut world = World::new();
        world.previous_client = true;

        issue(&world, "master").await.expect("issued");

        let calls = world.calls();
        assert_eq!(
            calls[1],
            step("DELETE", &format!("/realms/master/clients/{OLD}"))
        );
        assert_eq!(calls[2], step("POST", "/realms/master/clients"));
    }

    #[tokio::test]
    async fn the_client_is_a_confidential_service_account_without_redirects_or_password_grant() {
        let world = World::new();

        issue(&world, "master").await.expect("issued");

        let body = world
            .body_of(Method::POST, "/realms/master/clients")
            .expect("a body");
        assert_eq!(body["client_id"], "autharie-admin");
        assert_eq!(body["name"], "Autharie admin access");
        assert_eq!(body["client_type"], "confidential");
        assert_eq!(body["service_account_enabled"], true);
        assert_eq!(body["direct_access_grants_enabled"], false);
        assert_eq!(body["redirect_uris"], json!([]));
    }

    #[tokio::test]
    async fn the_role_grants_manage_realm_and_nothing_else() {
        let world = World::new();

        issue(&world, "master").await.expect("issued");

        let body = world
            .body_of(Method::POST, "/realms/master/roles")
            .expect("a body");
        assert_eq!(body["name"], "autharie-admin");
        assert_eq!(body["permissions"], json!(["manage_realm"]));
    }

    #[tokio::test]
    async fn a_role_already_there_is_kept_as_it_is() {
        let mut world = World::new();
        world.role = Some(json!({
            "id": ROLE, "name": "autharie-admin", "permissions": ["view_realm"]
        }));

        issue(&world, "master").await.expect("issued");

        let calls = world.calls();
        assert!(!calls.contains(&step("POST", "/realms/master/roles")));
        assert!(!calls.iter().any(|(_, path)| path.ends_with("/permissions")));
        assert!(calls.contains(&step(
            "POST",
            &format!("/realms/master/users/{USER}/roles/{ROLE}")
        )));
    }

    #[tokio::test]
    async fn another_realm_than_master_is_refused_before_any_call() {
        let world = World::new();

        let refused = issue(&world, "customers").await.unwrap_err();

        assert!(matches!(refused, AgentsError::Rejected { .. }));
        assert!(world.calls().is_empty());
    }

    #[tokio::test]
    async fn a_ferriskey_that_fails_midway_is_a_retry_and_hands_out_nothing() {
        let mut world = World::new();
        world.fail_on = Some((Method::POST, "/roles"));

        let failed = issue(&world, "master").await.unwrap_err();

        assert!(matches!(failed, AgentsError::Retry { .. }));
    }

    #[tokio::test]
    async fn a_client_without_a_secret_is_not_issued() {
        struct Bare(World);
        impl Caller for Bare {
            async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
                if request.path.ends_with("/client-secret") {
                    return Ok(json!({ "data": {} }));
                }
                self.0.call(request).await
            }
        }

        assert!(issue(&Bare(World::new()), "master").await.is_err());
    }
}
