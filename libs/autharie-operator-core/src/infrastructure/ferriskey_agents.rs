use std::future::Future;
use std::sync::Mutex;
use std::time::Instant;

use kube::Client;
use reqwest::Method;
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::agents::ports::FerriskeyAgentsPort;
use crate::domain::agents::{
    AgentsError, GATEWAY_CLIENT_ID, GatewayClient, MANAGED_BY, ManagedPolicy, PolicyWrite,
    RealmSettings, RemotePolicy, evaluator_audience,
};

use super::ferriskey_admin::{
    AdminCredentials, api_base_url, name_and_namespace, read_credentials,
};
use super::ferriskey_webhooks::{
    ApiFailure, ApiRequest, FerriskeyWebhookApi, MAX_PAGES, PAGE_LIMIT, TokenCache, next_page,
};

const EVALUATOR_ROLE: &str = "authz-evaluator";
const EVALUATE_PERMISSION: &str = "evaluate_authorization";

trait Caller: Send + Sync {
    fn call(&self, request: &ApiRequest) -> impl Future<Output = Result<Value, ApiFailure>> + Send;
}

fn failure(failure: ApiFailure) -> AgentsError {
    match failure {
        ApiFailure::Unauthorized => AgentsError::Retry {
            message: "FerrisKey refused the admin token".to_string(),
        },
        ApiFailure::Retry(message) => AgentsError::Retry { message },
        ApiFailure::Rejected { status: 404, .. } => AgentsError::NotFound,
        ApiFailure::Rejected { message, .. } => AgentsError::Rejected { message },
    }
}

async fn send<C: Caller>(caller: &C, request: ApiRequest) -> Result<Value, AgentsError> {
    caller.call(&request).await.map_err(failure)
}

fn malformed(what: &str) -> AgentsError {
    AgentsError::Retry {
        message: format!("FerrisKey answered {what} in a shape the operator does not know"),
    }
}

fn segment(value: &str) -> Result<&str, AgentsError> {
    let slug = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if slug && value != "." && value != ".." {
        Ok(value)
    } else {
        Err(AgentsError::Rejected {
            message: format!("{value:?} is not a valid name"),
        })
    }
}

fn items(body: &Value) -> Vec<&Value> {
    body["data"]
        .as_array()
        .or_else(|| body.as_array())
        .map(|list| list.iter().collect())
        .unwrap_or_default()
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key].as_str()
}

fn id_of(value: &Value, what: &str) -> Result<String, AgentsError> {
    text(value, "id")
        .or_else(|| text(&value["data"], "id"))
        .map(ToOwned::to_owned)
        .ok_or_else(|| malformed(what))
}

pub fn authorization_state_request(realm: &str, enabled: bool) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::PUT,
        format!("/realms/{}/authz/state", segment(realm)?),
    )
    .body(json!({ "enabled": enabled })))
}

pub fn realm_request(realm: &str) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::GET,
        format!("/realms/{}", segment(realm)?),
    ))
}

pub fn parse_realm_settings(body: &Value) -> RealmSettings {
    let settings = if body["settings"].is_object() {
        &body["settings"]
    } else {
        &body["data"]["settings"]
    };
    RealmSettings {
        allowed_resources: settings["allowed_resources"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        cimd_enabled: settings["cimd_enabled"].as_bool().unwrap_or(false),
        dcr_enabled: settings["dcr_enabled"].as_bool().unwrap_or(false),
    }
}

pub fn settings_request(realm: &str, settings: &RealmSettings) -> Result<ApiRequest, AgentsError> {
    Ok(
        ApiRequest::new(Method::PUT, format!("/realms/{}/settings", segment(realm)?)).body(json!({
            "allowed_resources": settings.allowed_resources,
            "cimd_enabled": settings.cimd_enabled,
            "dcr_enabled": settings.dcr_enabled,
        })),
    )
}

fn policies_path(realm: &str) -> Result<String, AgentsError> {
    Ok(format!("/realms/{}/authz/policies", segment(realm)?))
}

pub fn list_policies_request(realm: &str, page: u32) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(Method::GET, policies_path(realm)?)
        .query("managed_by", MANAGED_BY)
        .query("page", page)
        .query("limit", PAGE_LIMIT))
}

fn policy_body(write: &PolicyWrite) -> Value {
    json!({
        "name": write.name,
        "description": write.description,
        "enabled": write.enabled,
        "managed_by": MANAGED_BY,
        "body": { "type": "static", "source": write.source },
    })
}

pub fn create_policy_request(realm: &str, write: &PolicyWrite) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(Method::POST, policies_path(realm)?).body(policy_body(write)))
}

pub fn update_policy_request(
    realm: &str,
    ferriskey_id: &str,
    write: &PolicyWrite,
) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::PUT,
        format!("{}/{}", policies_path(realm)?, segment(ferriskey_id)?),
    )
    .body(policy_body(write)))
}

pub fn delete_policy_request(realm: &str, ferriskey_id: &str) -> Result<ApiRequest, AgentsError> {
    Ok(ApiRequest::new(
        Method::DELETE,
        format!("{}/{}", policies_path(realm)?, segment(ferriskey_id)?),
    )
    .query("managed_by", MANAGED_BY))
}

pub fn parse_policy(value: &Value) -> Result<RemotePolicy, AgentsError> {
    let source = text(&value["body"], "source")
        .or_else(|| text(value, "source"))
        .or_else(|| text(value, "cedar"))
        .unwrap_or_default();
    Ok(RemotePolicy {
        ferriskey_id: id_of(value, "a policy")?,
        name: text(value, "name")
            .ok_or_else(|| malformed("a policy"))?
            .to_string(),
        description: text(value, "description").map(ToOwned::to_owned),
        source: source.to_string(),
        enabled: value["enabled"].as_bool().unwrap_or(true),
    })
}

pub fn parse_policies(body: &Value) -> Result<(Vec<RemotePolicy>, Option<u32>), AgentsError> {
    let policies = items(body)
        .into_iter()
        .map(parse_policy)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((policies, next_page(body)))
}

struct RemoteClient {
    id: String,
    enabled: bool,
    token_exchange_enabled: bool,
}

fn parse_client(value: &Value) -> Result<RemoteClient, AgentsError> {
    Ok(RemoteClient {
        id: id_of(value, "a client")?,
        enabled: value["enabled"].as_bool().unwrap_or(true),
        token_exchange_enabled: value["token_exchange_enabled"].as_bool().unwrap_or(false),
    })
}

fn client_path(realm: &str, client: &str) -> Result<String, AgentsError> {
    Ok(format!(
        "/realms/{}/clients/{}",
        segment(realm)?,
        segment(client)?
    ))
}

async fn find_client<C: Caller>(
    caller: &C,
    realm: &str,
) -> Result<Option<RemoteClient>, AgentsError> {
    let body = send(
        caller,
        ApiRequest::new(Method::GET, format!("/realms/{}/clients", segment(realm)?))
            .query("client_id_exact", GATEWAY_CLIENT_ID)
            .query("limit", 10),
    )
    .await?;
    items(&body)
        .into_iter()
        .find(|client| text(client, "client_id") == Some(GATEWAY_CLIENT_ID))
        .map(parse_client)
        .transpose()
}

async fn ensure_client<C: Caller>(caller: &C, realm: &str) -> Result<RemoteClient, AgentsError> {
    match find_client(caller, realm).await? {
        Some(client) if client.enabled && client.token_exchange_enabled => Ok(client),
        Some(client) => {
            send(
                caller,
                ApiRequest::new(Method::PATCH, client_path(realm, &client.id)?)
                    .body(json!({ "enabled": true, "token_exchange_enabled": true })),
            )
            .await?;
            Ok(RemoteClient {
                enabled: true,
                token_exchange_enabled: true,
                ..client
            })
        }
        None => {
            let created = send(
                caller,
                ApiRequest::new(Method::POST, format!("/realms/{}/clients", segment(realm)?)).body(
                    json!({
                        "name": "MCP gateway",
                        "client_id": GATEWAY_CLIENT_ID,
                        "client_type": "confidential",
                        "service_account_enabled": true,
                        "public_client": false,
                        "enabled": true,
                        "direct_access_grants_enabled": false,
                        "token_exchange_enabled": true,
                        "require_pkce": false,
                    }),
                ),
            )
            .await?;
            parse_client(&created)
        }
    }
}

async fn client_secret<C: Caller>(
    caller: &C,
    realm: &str,
    client: &RemoteClient,
) -> Result<String, AgentsError> {
    let body = send(
        caller,
        ApiRequest::new(
            Method::GET,
            format!("{}/client-secret", client_path(realm, &client.id)?),
        ),
    )
    .await?;
    text(&body, "client_secret")
        .or_else(|| text(&body["data"], "client_secret"))
        .filter(|secret| !secret.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| malformed("a client without a secret"))
}

async fn ensure_evaluator_role<C: Caller>(
    caller: &C,
    realm: &str,
    client: &RemoteClient,
) -> Result<String, AgentsError> {
    let roles_path = format!("{}/roles", client_path(realm, &client.id)?);
    let body = send(caller, ApiRequest::new(Method::GET, roles_path.clone())).await?;
    let existing = items(&body)
        .into_iter()
        .find(|role| text(role, "name") == Some(EVALUATOR_ROLE));

    match existing {
        Some(role) => {
            let id = id_of(role, "a role")?;
            let permitted = role["permissions"]
                .as_array()
                .is_some_and(|list| list.iter().any(|p| p.as_str() == Some(EVALUATE_PERMISSION)));
            if !permitted {
                send(
                    caller,
                    ApiRequest::new(
                        Method::PATCH,
                        format!(
                            "/realms/{}/roles/{}/permissions",
                            segment(realm)?,
                            segment(&id)?
                        ),
                    )
                    .body(json!({ "permissions": [EVALUATE_PERMISSION] })),
                )
                .await?;
            }
            Ok(id)
        }
        None => {
            let created = send(
                caller,
                ApiRequest::new(Method::POST, roles_path).body(json!({
                    "name": EVALUATOR_ROLE,
                    "description": "Lets the MCP gateway ask this realm for authorization decisions",
                    "permissions": [EVALUATE_PERMISSION],
                })),
            )
            .await?;
            id_of(&created, "a role")
        }
    }
}

async fn service_account<C: Caller>(
    caller: &C,
    realm: &str,
    client: &RemoteClient,
) -> Result<String, AgentsError> {
    let username = format!("service-account-{GATEWAY_CLIENT_ID}");
    let body = send(
        caller,
        ApiRequest::new(Method::GET, format!("/realms/{}/users", segment(realm)?))
            .query("username", &username)
            .query("service_account", true)
            .query("limit", PAGE_LIMIT),
    )
    .await?;
    let found = items(&body).into_iter().find(|user| {
        text(user, "username") == Some(username.as_str())
            && text(user, "client_id").is_none_or(|owner| owner == client.id)
    });
    match found {
        Some(user) => id_of(user, "a user"),
        None => Err(AgentsError::Retry {
            message: "the gateway client's service account does not exist yet".to_string(),
        }),
    }
}

async fn ensure_role_assigned<C: Caller>(
    caller: &C,
    realm: &str,
    user: &str,
    role: &str,
) -> Result<(), AgentsError> {
    let user_path = format!("/realms/{}/users/{}", segment(realm)?, segment(user)?);
    let body = send(
        caller,
        ApiRequest::new(Method::GET, format!("{user_path}/roles")),
    )
    .await?;
    if items(&body)
        .into_iter()
        .any(|assigned| text(assigned, "id") == Some(role))
    {
        return Ok(());
    }
    send(
        caller,
        ApiRequest::new(
            Method::POST,
            format!("{user_path}/roles/{}", segment(role)?),
        ),
    )
    .await
    .map(|_| ())
}

async fn ensure_exchange_policy<C: Caller>(
    caller: &C,
    realm: &str,
    client: &RemoteClient,
) -> Result<(), AgentsError> {
    let audience = evaluator_audience(realm);
    let policies_path = format!(
        "{}/token-exchange-policies",
        client_path(realm, &client.id)?
    );
    let body = send(caller, ApiRequest::new(Method::GET, policies_path.clone())).await?;
    let existing = items(&body)
        .into_iter()
        .find(|policy| text(policy, "target_audience") == Some(audience.as_str()));

    if let Some(policy) = existing {
        if policy["allow_delegation"].as_bool().unwrap_or(false) {
            return Ok(());
        }
        let id = id_of(policy, "a token exchange policy")?;
        send(
            caller,
            ApiRequest::new(Method::DELETE, format!("{policies_path}/{}", segment(&id)?)),
        )
        .await?;
    }

    send(
        caller,
        ApiRequest::new(Method::POST, policies_path).body(json!({
            "target_audience": audience,
            "allow_delegation": true,
            "allow_impersonation": false,
        })),
    )
    .await
    .map(|_| ())
}

struct Owner {
    uri: String,
    client_id: String,
}

async fn resource_owners<C: Caller>(caller: &C, realm: &str) -> Result<Vec<Owner>, AgentsError> {
    let body = send(
        caller,
        ApiRequest::new(
            Method::GET,
            format!("/realms/{}/resource-owners", segment(realm)?),
        ),
    )
    .await?;
    Ok(items(&body)
        .into_iter()
        .filter_map(|owner| {
            Some(Owner {
                uri: text(owner, "uri")?.to_string(),
                client_id: text(owner, "client_id")?.to_string(),
            })
        })
        .collect())
}

async fn replace_owners<C: Caller>(
    caller: &C,
    realm: &str,
    owners: &[Owner],
) -> Result<(), AgentsError> {
    let listed: Vec<Value> = owners
        .iter()
        .map(|owner| json!({ "uri": owner.uri, "client_id": owner.client_id }))
        .collect();
    send(
        caller,
        ApiRequest::new(
            Method::PUT,
            format!("/realms/{}/resource-owners", segment(realm)?),
        )
        .body(json!({ "owners": listed })),
    )
    .await
    .map(|_| ())
}

async fn ensure_owner<C: Caller>(
    caller: &C,
    realm: &str,
    client: &RemoteClient,
    canonical: &str,
) -> Result<(), AgentsError> {
    let owners = resource_owners(caller, realm).await?;
    if owners
        .iter()
        .any(|owner| owner.uri == canonical && owner.client_id == client.id)
    {
        return Ok(());
    }
    let mut next: Vec<Owner> = owners
        .into_iter()
        .filter(|owner| owner.uri != canonical)
        .collect();
    next.push(Owner {
        uri: canonical.to_string(),
        client_id: client.id.clone(),
    });
    replace_owners(caller, realm, &next).await
}

async fn ensure_gateway_client<C: Caller>(
    caller: &C,
    realm: &str,
    canonical: &str,
) -> Result<GatewayClient, AgentsError> {
    let client = ensure_client(caller, realm).await?;
    let secret = client_secret(caller, realm, &client).await?;
    let role = ensure_evaluator_role(caller, realm, &client).await?;
    let user = service_account(caller, realm, &client).await?;
    ensure_role_assigned(caller, realm, &user, &role).await?;
    ensure_exchange_policy(caller, realm, &client).await?;
    ensure_owner(caller, realm, &client, canonical).await?;
    Ok(GatewayClient { secret })
}

async fn remove_gateway_client<C: Caller>(
    caller: &C,
    realm: &str,
    canonical: &str,
) -> Result<(), AgentsError> {
    let owners = resource_owners(caller, realm).await?;
    if owners.iter().any(|owner| owner.uri == canonical) {
        let kept: Vec<Owner> = owners
            .into_iter()
            .filter(|owner| owner.uri != canonical)
            .collect();
        replace_owners(caller, realm, &kept).await?;
    }

    if let Some(client) = find_client(caller, realm).await? {
        send(
            caller,
            ApiRequest::new(Method::DELETE, client_path(realm, &client.id)?),
        )
        .await?;
    }
    Ok(())
}

async fn all_policies<C: Caller>(
    caller: &C,
    realm: &str,
) -> Result<Vec<RemotePolicy>, AgentsError> {
    let mut all = Vec::new();
    let mut page = 1;
    for _ in 0..MAX_PAGES {
        let body = send(caller, list_policies_request(realm, page)?).await?;
        let (mut found, next) = parse_policies(&body)?;
        all.append(&mut found);
        match next {
            Some(next) if next > page => page = next,
            _ => break,
        }
    }
    Ok(all)
}

pub struct KubeFerriskeyAgents {
    client: Client,
    api: FerriskeyWebhookApi,
    tokens: Mutex<TokenCache>,
}

impl KubeFerriskeyAgents {
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

    async fn call_instance(
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

    fn bound<'a>(&'a self, instance: &'a IdentityInstance) -> Bound<'a> {
        Bound {
            owner: self,
            instance,
        }
    }
}

struct Bound<'a> {
    owner: &'a KubeFerriskeyAgents,
    instance: &'a IdentityInstance,
}

impl Caller for Bound<'_> {
    async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
        self.owner.call_instance(self.instance, request).await
    }
}

impl FerriskeyAgentsPort for KubeFerriskeyAgents {
    async fn set_authorization(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        enabled: bool,
    ) -> Result<(), AgentsError> {
        match send(
            &self.bound(instance),
            authorization_state_request(realm, enabled)?,
        )
        .await
        {
            Ok(_) => Ok(()),
            Err(AgentsError::NotFound) => Err(AgentsError::Retry {
                message: "FerrisKey's authorization API is not available yet".to_string(),
            }),
            Err(error) => Err(error),
        }
    }

    async fn realm_settings(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> Result<RealmSettings, AgentsError> {
        let body = send(&self.bound(instance), realm_request(realm)?).await?;
        Ok(parse_realm_settings(&body))
    }

    async fn update_realm_settings(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        settings: &RealmSettings,
    ) -> Result<(), AgentsError> {
        send(&self.bound(instance), settings_request(realm, settings)?)
            .await
            .map(|_| ())
    }

    async fn ensure_gateway_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        canonical_uri: &str,
    ) -> Result<GatewayClient, AgentsError> {
        ensure_gateway_client(&self.bound(instance), realm, canonical_uri).await
    }

    async fn remove_gateway_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        canonical_uri: &str,
    ) -> Result<(), AgentsError> {
        remove_gateway_client(&self.bound(instance), realm, canonical_uri).await
    }

    async fn list_managed_policies(
        &self,
        instance: &IdentityInstance,
        realm: &str,
    ) -> Result<Vec<RemotePolicy>, AgentsError> {
        all_policies(&self.bound(instance), realm).await
    }

    async fn create_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        write: &PolicyWrite,
    ) -> Result<(), AgentsError> {
        send(&self.bound(instance), create_policy_request(realm, write)?)
            .await
            .map(|_| ())
    }

    async fn update_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        policy: &ManagedPolicy,
        write: &PolicyWrite,
    ) -> Result<(), AgentsError> {
        send(
            &self.bound(instance),
            update_policy_request(realm, &policy.policy.ferriskey_id, write)?,
        )
        .await
        .map(|_| ())
    }

    async fn delete_policy(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        policy: &ManagedPolicy,
    ) -> Result<(), AgentsError> {
        send(
            &self.bound(instance),
            delete_policy_request(realm, &policy.policy.ferriskey_id)?,
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const CANONICAL: &str = "https://auth.acme.test/mcp";

    struct World {
        routes: Vec<(Method, String, Value)>,
        log: Mutex<Vec<(Method, String, Option<Value>)>>,
    }

    impl World {
        fn new(routes: Vec<(Method, &str, Value)>) -> Self {
            Self {
                routes: routes
                    .into_iter()
                    .map(|(method, path, body)| (method, path.to_string(), body))
                    .collect(),
                log: Mutex::new(Vec::new()),
            }
        }

        fn writes(&self) -> Vec<(Method, String, Option<Value>)> {
            self.log
                .lock()
                .expect("not poisoned")
                .iter()
                .filter(|(method, _, _)| method != Method::GET)
                .cloned()
                .collect()
        }

        fn wrote(&self, method: Method, path: &str) -> Option<Value> {
            self.writes()
                .into_iter()
                .find(|(m, p, _)| *m == method && p == path)
                .map(|(_, _, body)| body.unwrap_or(Value::Null))
        }
    }

    impl Caller for World {
        async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
            self.log.lock().expect("not poisoned").push((
                request.method.clone(),
                request.path.clone(),
                request.body.clone(),
            ));
            self.routes
                .iter()
                .find(|(method, path, _)| *method == request.method && *path == request.path)
                .map(|(_, _, body)| body.clone())
                .ok_or(ApiFailure::Rejected {
                    status: 404,
                    message: "FerrisKey answered 404".to_string(),
                })
        }
    }

    fn fresh_realm() -> World {
        World::new(vec![
            (Method::GET, "/realms/acme/clients", json!({ "data": [] })),
            (
                Method::POST,
                "/realms/acme/clients",
                json!({ "id": "c1", "client_id": "mcp-gateway", "client_secret": "created" }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/client-secret",
                json!({ "client_secret": "s3cret" }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/roles",
                json!({ "data": [] }),
            ),
            (
                Method::POST,
                "/realms/acme/clients/c1/roles",
                json!({ "id": "r1", "name": "authz-evaluator" }),
            ),
            (
                Method::GET,
                "/realms/acme/users",
                json!({ "data": [
                    { "id": "u0", "username": "service-account-mcp-gateway-2", "client_id": "other" },
                    { "id": "u1", "username": "service-account-mcp-gateway", "client_id": "c1" }
                ] }),
            ),
            (
                Method::GET,
                "/realms/acme/users/u1/roles",
                json!({ "data": [] }),
            ),
            (Method::POST, "/realms/acme/users/u1/roles/r1", json!({})),
            (
                Method::GET,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!([]),
            ),
            (
                Method::POST,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!({ "id": "x1" }),
            ),
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [{ "uri": "https://other.test/mcp", "client_id": "zz" }] }),
            ),
            (Method::PUT, "/realms/acme/resource-owners", json!({})),
        ])
    }

    fn configured_realm() -> World {
        World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients",
                json!({ "data": [{
                    "id": "c1", "client_id": "mcp-gateway", "enabled": true,
                    "token_exchange_enabled": true
                }] }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/client-secret",
                json!({ "client_secret": "s3cret" }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/roles",
                json!({ "data": [{
                    "id": "r1", "name": "authz-evaluator",
                    "permissions": ["evaluate_authorization"]
                }] }),
            ),
            (
                Method::GET,
                "/realms/acme/users",
                json!({ "data": [{
                    "id": "u1", "username": "service-account-mcp-gateway", "client_id": "c1"
                }] }),
            ),
            (
                Method::GET,
                "/realms/acme/users/u1/roles",
                json!({ "data": [{ "id": "r1" }] }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!([{ "id": "x1", "target_audience": "acme-realm", "allow_delegation": true }]),
            ),
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [{ "uri": CANONICAL, "client_id": "c1" }] }),
            ),
        ])
    }

    #[tokio::test]
    async fn a_fresh_realm_gets_a_confidential_service_account_client() {
        let world = fresh_realm();

        let client = ensure_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("ensured");

        assert_eq!(client.secret, "s3cret");
        let body = world
            .wrote(Method::POST, "/realms/acme/clients")
            .expect("a client was created");
        assert_eq!(body["client_id"], "mcp-gateway");
        assert_eq!(body["client_type"], "confidential");
        assert_eq!(body["service_account_enabled"], true);
        assert_eq!(body["token_exchange_enabled"], true);
        assert_eq!(body["public_client"], false);
        assert_eq!(body["enabled"], true);
    }

    #[tokio::test]
    async fn the_service_account_gets_the_evaluate_authorization_permission() {
        let world = fresh_realm();

        ensure_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("ensured");

        let role = world
            .wrote(Method::POST, "/realms/acme/clients/c1/roles")
            .expect("a role was created");
        assert_eq!(role["permissions"], json!(["evaluate_authorization"]));
        assert!(
            world
                .wrote(Method::POST, "/realms/acme/users/u1/roles/r1")
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_gateway_may_delegate_to_the_realm_api_audience_and_nothing_else() {
        let world = fresh_realm();

        ensure_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("ensured");

        let policy = world
            .wrote(
                Method::POST,
                "/realms/acme/clients/c1/token-exchange-policies",
            )
            .expect("a policy was created");
        assert_eq!(policy["target_audience"], "acme-realm");
        assert_eq!(policy["allow_delegation"], true);
        assert_eq!(policy["allow_impersonation"], false);
    }

    #[tokio::test]
    async fn the_gateway_client_owns_the_canonical_uri_beside_the_owners_already_there() {
        let world = fresh_realm();

        ensure_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("ensured");

        let owners = world
            .wrote(Method::PUT, "/realms/acme/resource-owners")
            .expect("owners were replaced");
        assert_eq!(
            owners,
            json!({ "owners": [
                { "uri": "https://other.test/mcp", "client_id": "zz" },
                { "uri": CANONICAL, "client_id": "c1" }
            ] })
        );
    }

    #[tokio::test]
    async fn a_realm_already_set_up_is_only_read() {
        let world = configured_realm();

        let client = ensure_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("ensured");

        assert_eq!(client.secret, "s3cret");
        assert_eq!(world.writes(), Vec::new());
    }

    #[tokio::test]
    async fn a_client_somebody_disabled_is_enabled_again() {
        let mut routes = vec![(
            Method::GET,
            "/realms/acme/clients",
            json!({ "data": [{
                "id": "c1", "client_id": "mcp-gateway", "enabled": false,
                "token_exchange_enabled": false
            }] }),
        )];
        routes.push((Method::PATCH, "/realms/acme/clients/c1", json!({})));
        let world = World::new(routes);

        let client = ensure_client(&world, "acme").await.expect("ensured");

        assert!(client.enabled && client.token_exchange_enabled);
        assert_eq!(
            world.wrote(Method::PATCH, "/realms/acme/clients/c1"),
            Some(json!({ "enabled": true, "token_exchange_enabled": true }))
        );
    }

    #[tokio::test]
    async fn an_exchange_policy_that_does_not_delegate_is_replaced() {
        let world = World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!([{ "id": "x1", "target_audience": "acme-realm", "allow_delegation": false }]),
            ),
            (
                Method::DELETE,
                "/realms/acme/clients/c1/token-exchange-policies/x1",
                json!({}),
            ),
            (
                Method::POST,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!({}),
            ),
        ]);
        let client = RemoteClient {
            id: "c1".to_string(),
            enabled: true,
            token_exchange_enabled: true,
        };

        ensure_exchange_policy(&world, "acme", &client)
            .await
            .expect("ensured");

        let methods: Vec<_> = world.writes().into_iter().map(|(m, _, _)| m).collect();
        assert_eq!(methods, vec![Method::DELETE, Method::POST]);
    }

    #[tokio::test]
    async fn a_role_missing_the_permission_is_given_it() {
        let world = World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients/c1/roles",
                json!({ "data": [{ "id": "r1", "name": "authz-evaluator", "permissions": [] }] }),
            ),
            (
                Method::PATCH,
                "/realms/acme/roles/r1/permissions",
                json!({}),
            ),
        ]);
        let client = RemoteClient {
            id: "c1".to_string(),
            enabled: true,
            token_exchange_enabled: true,
        };

        let role = ensure_evaluator_role(&world, "acme", &client)
            .await
            .expect("ensured");

        assert_eq!(role, "r1");
        assert_eq!(
            world.wrote(Method::PATCH, "/realms/acme/roles/r1/permissions"),
            Some(json!({ "permissions": ["evaluate_authorization"] }))
        );
    }

    #[tokio::test]
    async fn a_service_account_not_there_yet_is_a_reason_to_wait() {
        let world = World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients",
                json!({ "data": [{ "id": "c1", "client_id": "mcp-gateway" }] }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/client-secret",
                json!({ "client_secret": "s" }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/roles",
                json!({ "data": [{ "id": "r1", "name": "authz-evaluator", "permissions": ["evaluate_authorization"] }] }),
            ),
            (Method::GET, "/realms/acme/users", json!({ "data": [] })),
            (Method::PATCH, "/realms/acme/clients/c1", json!({})),
        ]);

        let error = ensure_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect_err("waits");

        assert!(error.is_retry());
    }

    #[tokio::test]
    async fn a_client_without_a_secret_is_not_ready() {
        let world = World::new(vec![(
            Method::GET,
            "/realms/acme/clients/c1/client-secret",
            json!({ "client_secret": null }),
        )]);
        let client = RemoteClient {
            id: "c1".to_string(),
            enabled: true,
            token_exchange_enabled: true,
        };

        assert!(
            client_secret(&world, "acme", &client)
                .await
                .expect_err("no secret")
                .is_retry()
        );
    }

    #[tokio::test]
    async fn removing_the_client_takes_its_ownership_and_leaves_the_others() {
        let world = World::new(vec![
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [
                    { "uri": CANONICAL, "client_id": "c1" },
                    { "uri": "https://other.test/mcp", "client_id": "zz" }
                ] }),
            ),
            (Method::PUT, "/realms/acme/resource-owners", json!({})),
            (
                Method::GET,
                "/realms/acme/clients",
                json!({ "data": [{ "id": "c1", "client_id": "mcp-gateway" }] }),
            ),
            (Method::DELETE, "/realms/acme/clients/c1", json!({})),
        ]);

        remove_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("removed");

        assert_eq!(
            world.wrote(Method::PUT, "/realms/acme/resource-owners"),
            Some(json!({ "owners": [{ "uri": "https://other.test/mcp", "client_id": "zz" }] }))
        );
        assert!(
            world
                .wrote(Method::DELETE, "/realms/acme/clients/c1")
                .is_some()
        );
    }

    #[tokio::test]
    async fn removing_what_is_not_there_writes_nothing() {
        let world = World::new(vec![
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [] }),
            ),
            (Method::GET, "/realms/acme/clients", json!({ "data": [] })),
        ]);

        remove_gateway_client(&world, "acme", CANONICAL)
            .await
            .expect("removed");

        assert_eq!(world.writes(), Vec::new());
    }

    #[tokio::test]
    async fn every_page_of_policies_is_read() {
        struct Pages;
        impl Caller for Pages {
            async fn call(&self, request: &ApiRequest) -> Result<Value, ApiFailure> {
                let page = request
                    .query
                    .iter()
                    .find(|(key, _)| *key == "page")
                    .map(|(_, value)| value.as_str())
                    .unwrap_or("1");
                Ok(match page {
                    "1" => json!({
                        "data": [{ "id": "p1", "name": "autharie:a", "body": { "source": "x" } }],
                        "metadata": { "next_page": 2 }
                    }),
                    _ => json!({
                        "data": [{ "id": "p2", "name": "autharie:b", "source": "y" }],
                        "metadata": { "next_page": null }
                    }),
                })
            }
        }

        let found = all_policies(&Pages, "acme").await.expect("listed");

        assert_eq!(found.len(), 2);
        assert_eq!(found[1].source, "y");
    }

    #[test]
    fn listing_asks_only_for_what_autharie_manages() {
        let request = list_policies_request("acme", 2).expect("a request");

        assert_eq!(request.method, Method::GET);
        assert_eq!(request.path, "/realms/acme/authz/policies");
        assert!(
            request
                .query
                .contains(&("managed_by", "autharie".to_string()))
        );
        assert!(request.query.contains(&("page", "2".to_string())));
    }

    fn write() -> PolicyWrite {
        PolicyWrite {
            name: "autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11".to_string(),
            description: "Read only".to_string(),
            source: "permit (principal, action, resource);".to_string(),
            enabled: true,
        }
    }

    #[test]
    fn a_policy_is_written_as_a_labelled_static_policy() {
        let request = create_policy_request("acme", &write()).expect("a request");

        assert_eq!(request.method, Method::POST);
        assert_eq!(request.path, "/realms/acme/authz/policies");
        assert_eq!(
            request.body,
            Some(json!({
                "name": "autharie:0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
                "description": "Read only",
                "enabled": true,
                "managed_by": "autharie",
                "body": { "type": "static", "source": "permit (principal, action, resource);" },
            }))
        );
    }

    #[test]
    fn an_update_and_a_delete_name_the_policy_and_the_manager() {
        let update = update_policy_request("acme", "f1", &write()).expect("a request");
        let delete = delete_policy_request("acme", "f1").expect("a request");

        assert_eq!(update.method, Method::PUT);
        assert_eq!(update.path, "/realms/acme/authz/policies/f1");
        assert_eq!(update.body.as_ref().unwrap()["managed_by"], "autharie");
        assert_eq!(delete.method, Method::DELETE);
        assert_eq!(delete.path, "/realms/acme/authz/policies/f1");
        assert!(
            delete
                .query
                .contains(&("managed_by", "autharie".to_string()))
        );
    }

    #[test]
    fn a_name_that_could_change_the_path_is_refused() {
        assert!(create_policy_request("../master", &write()).is_err());
        assert!(delete_policy_request("acme", "a/b").is_err());
        assert!(authorization_state_request("a b", true).is_err());
    }

    #[test]
    fn a_policy_is_read_from_its_body_or_its_flat_fields() {
        let nested = parse_policy(&json!({
            "id": "p1", "name": "autharie:a", "description": "d", "enabled": false,
            "body": { "type": "static", "source": "permit;" }
        }))
        .expect("parsed");
        let flat =
            parse_policy(&json!({ "id": "p2", "name": "n", "cedar": "forbid;" })).expect("parsed");

        assert_eq!(nested.source, "permit;");
        assert!(!nested.enabled);
        assert_eq!(nested.description.as_deref(), Some("d"));
        assert_eq!(flat.source, "forbid;");
        assert!(flat.enabled);
        assert!(parse_policy(&json!({ "name": "n" })).is_err());
    }

    #[test]
    fn the_authorization_state_is_a_single_switch() {
        let request = authorization_state_request("acme", true).expect("a request");

        assert_eq!(request.method, Method::PUT);
        assert_eq!(request.path, "/realms/acme/authz/state");
        assert_eq!(request.body, Some(json!({ "enabled": true })));
    }

    #[test]
    fn realm_settings_are_read_whether_or_not_wrapped() {
        let flat = json!({ "settings": {
            "allowed_resources": ["https://a/mcp"], "cimd_enabled": true, "dcr_enabled": false
        } });
        let wrapped = json!({ "data": flat.clone() });

        for body in [flat, wrapped] {
            assert_eq!(
                parse_realm_settings(&body),
                RealmSettings {
                    allowed_resources: vec!["https://a/mcp".to_string()],
                    cimd_enabled: true,
                    dcr_enabled: false,
                }
            );
        }
        assert_eq!(parse_realm_settings(&json!({})), RealmSettings::default());
    }

    #[test]
    fn settings_are_written_whole_for_the_three_fields_the_operator_owns() {
        let request = settings_request(
            "acme",
            &RealmSettings {
                allowed_resources: vec![CANONICAL.to_string()],
                cimd_enabled: true,
                dcr_enabled: false,
            },
        )
        .expect("a request");

        assert_eq!(request.method, Method::PUT);
        assert_eq!(request.path, "/realms/acme/settings");
        assert_eq!(
            request.body,
            Some(json!({
                "allowed_resources": [CANONICAL],
                "cimd_enabled": true,
                "dcr_enabled": false,
            }))
        );
    }

    #[test]
    fn failures_keep_their_meaning() {
        assert_eq!(
            failure(ApiFailure::Rejected {
                status: 404,
                message: "m".to_string()
            }),
            AgentsError::NotFound
        );
        assert_eq!(
            failure(ApiFailure::Rejected {
                status: 422,
                message: "FerrisKey answered 422: bad".to_string()
            }),
            AgentsError::Rejected {
                message: "FerrisKey answered 422: bad".to_string()
            }
        );
        assert!(failure(ApiFailure::Retry("down".to_string())).is_retry());
        assert!(failure(ApiFailure::Unauthorized).is_retry());
    }
}
