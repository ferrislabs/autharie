use std::sync::Arc;

use reqwest::Method;

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use super::{
    Caller, KubeFerriskeyAgents, ensure_audience_client_for, ensure_exchange_policy_for,
    ensure_owner, find_client, id_of, items, release_owner, segment, send, text,
};
use crate::domain::agents::AgentsError;
use crate::domain::agents::servers::{
    AgentServersPort, Discovered, DiscoveryTarget, audience as audience_of,
};
use crate::infrastructure::ferriskey_webhooks::ApiRequest;
use crate::infrastructure::server_discovery::McpDiscoverer;

fn gateway_missing() -> AgentsError {
    AgentsError::Retry {
        message: "the gateway client does not exist yet".to_string(),
    }
}

async fn ensure_server<C: Caller>(
    caller: &C,
    realm: &str,
    slug: &str,
    canonical: &str,
) -> Result<(), AgentsError> {
    let gateway = find_client(caller, realm)
        .await?
        .ok_or_else(gateway_missing)?;
    let audience = audience_of(slug);

    ensure_audience_client_for(caller, realm, &audience, &format!("MCP server {slug}")).await?;
    ensure_exchange_policy_for(caller, realm, &gateway, &audience).await?;
    ensure_owner(caller, realm, &gateway, canonical).await
}

async fn remove_server<C: Caller>(
    caller: &C,
    realm: &str,
    slug: &str,
    canonical: &str,
) -> Result<(), AgentsError> {
    let audience = audience_of(slug);

    release_owner(caller, realm, canonical).await?;

    if let Some(gateway) = find_client(caller, realm).await? {
        let policies_path = format!(
            "/realms/{}/clients/{}/token-exchange-policies",
            segment(realm)?,
            segment(&gateway.id)?
        );
        let body = send(caller, ApiRequest::new(Method::GET, policies_path.clone())).await?;
        for policy in items(&body)
            .into_iter()
            .filter(|policy| text(policy, "target_audience") == Some(audience.as_str()))
        {
            let id = id_of(policy, "a token exchange policy")?;
            send(
                caller,
                ApiRequest::new(Method::DELETE, format!("{policies_path}/{}", segment(&id)?)),
            )
            .await?;
        }
    }

    let body = send(
        caller,
        ApiRequest::new(Method::GET, format!("/realms/{}/clients", segment(realm)?))
            .query("client_id_exact", audience.as_str())
            .query("limit", 10),
    )
    .await?;
    for client in items(&body)
        .into_iter()
        .filter(|client| text(client, "client_id") == Some(audience.as_str()))
    {
        let id = id_of(client, "a client")?;
        send(
            caller,
            ApiRequest::new(
                Method::DELETE,
                format!("/realms/{}/clients/{}", segment(realm)?, segment(&id)?),
            ),
        )
        .await?;
    }
    Ok(())
}

pub struct KubeAgentServers {
    agents: Arc<KubeFerriskeyAgents>,
    discoverer: McpDiscoverer,
}

impl KubeAgentServers {
    pub fn new(agents: Arc<KubeFerriskeyAgents>) -> Result<Self, reqwest::Error> {
        Ok(Self {
            agents,
            discoverer: McpDiscoverer::new()?,
        })
    }
}

impl AgentServersPort for KubeAgentServers {
    async fn ensure_server_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        slug: &str,
        canonical_uri: &str,
    ) -> Result<(), AgentsError> {
        ensure_server(&self.agents.bound(instance), realm, slug, canonical_uri).await
    }

    async fn remove_server_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        slug: &str,
        canonical_uri: &str,
    ) -> Result<(), AgentsError> {
        remove_server(&self.agents.bound(instance), realm, slug, canonical_uri).await
    }

    async fn discover(&self, target: &DiscoveryTarget<'_>) -> Discovered {
        self.discoverer.discover(target).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::infrastructure::ferriskey_webhooks::ApiFailure;

    const CRM_URI: &str = "https://auth.acme.test/mcp/crm";

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

    fn gateway() -> Value {
        json!({ "id": "c1", "client_id": "mcp-gateway", "enabled": true, "token_exchange_enabled": true })
    }

    fn fresh_server() -> World {
        World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients",
                json!({ "data": [gateway()] }),
            ),
            (
                Method::POST,
                "/realms/acme/clients",
                json!({ "id": "c9", "client_id": "mcp-crm" }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!([{ "id": "x1", "target_audience": "acme-realm", "allow_delegation": true }]),
            ),
            (
                Method::POST,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!({ "id": "x2" }),
            ),
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [{ "uri": "https://auth.acme.test/mcp", "client_id": "c1" }] }),
            ),
            (Method::PUT, "/realms/acme/resource-owners", json!({})),
        ])
    }

    fn set_up_server() -> World {
        World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients",
                json!({ "data": [gateway(), { "id": "c9", "client_id": "mcp-crm", "enabled": true }] }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!([
                    { "id": "x1", "target_audience": "acme-realm", "allow_delegation": true },
                    { "id": "x2", "target_audience": "mcp-crm", "allow_delegation": true }
                ]),
            ),
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [
                    { "uri": "https://auth.acme.test/mcp", "client_id": "c1" },
                    { "uri": CRM_URI, "client_id": "c1" }
                ] }),
            ),
        ])
    }

    #[tokio::test]
    async fn a_new_server_gets_an_audience_client_an_exchange_policy_and_an_owner() {
        let world = fresh_server();

        ensure_server(&world, "acme", "crm", CRM_URI)
            .await
            .expect("ready");

        let client = world
            .wrote(Method::POST, "/realms/acme/clients")
            .expect("a client");
        assert_eq!(client["client_id"], "mcp-crm");
        assert_eq!(client["service_account_enabled"], false);
        assert_eq!(client["token_exchange_enabled"], false);
        let policy = world
            .wrote(
                Method::POST,
                "/realms/acme/clients/c1/token-exchange-policies",
            )
            .expect("a policy");
        assert_eq!(policy["target_audience"], "mcp-crm");
        assert_eq!(policy["allow_delegation"], true);
        assert_eq!(policy["allow_impersonation"], false);
        let owners = world
            .wrote(Method::PUT, "/realms/acme/resource-owners")
            .expect("owners");
        let listed: Vec<_> = owners["owners"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|owner| (owner["uri"].clone(), owner["client_id"].clone()))
            .collect();
        assert_eq!(
            listed,
            vec![
                (json!("https://auth.acme.test/mcp"), json!("c1")),
                (json!(CRM_URI), json!("c1")),
            ]
        );
    }

    #[tokio::test]
    async fn a_server_already_set_up_is_only_read() {
        let world = set_up_server();

        ensure_server(&world, "acme", "crm", CRM_URI)
            .await
            .expect("ready");

        assert_eq!(world.writes(), Vec::new());
    }

    #[tokio::test]
    async fn without_the_gateway_client_a_server_waits() {
        let world = World::new(vec![(
            Method::GET,
            "/realms/acme/clients",
            json!({ "data": [] }),
        )]);

        let error = ensure_server(&world, "acme", "crm", CRM_URI)
            .await
            .expect_err("waits");

        assert!(error.is_retry());
        assert_eq!(world.writes(), Vec::new());
    }

    #[tokio::test]
    async fn removing_a_server_takes_its_owner_policy_and_client_and_nothing_else() {
        let mut routes = set_up_server();
        routes.routes.extend([
            (
                Method::PUT,
                "/realms/acme/resource-owners".to_string(),
                json!({}),
            ),
            (
                Method::DELETE,
                "/realms/acme/clients/c1/token-exchange-policies/x2".to_string(),
                json!({}),
            ),
            (
                Method::DELETE,
                "/realms/acme/clients/c9".to_string(),
                json!({}),
            ),
        ]);

        remove_server(&routes, "acme", "crm", CRM_URI)
            .await
            .expect("removed");

        let owners = routes
            .wrote(Method::PUT, "/realms/acme/resource-owners")
            .expect("owners");
        assert_eq!(owners["owners"].as_array().expect("a list").len(), 1);
        assert_eq!(owners["owners"][0]["uri"], "https://auth.acme.test/mcp");
        assert!(
            routes
                .wrote(
                    Method::DELETE,
                    "/realms/acme/clients/c1/token-exchange-policies/x2"
                )
                .is_some()
        );
        assert!(
            routes
                .wrote(
                    Method::DELETE,
                    "/realms/acme/clients/c1/token-exchange-policies/x1"
                )
                .is_none()
        );
        assert!(
            routes
                .wrote(Method::DELETE, "/realms/acme/clients/c9")
                .is_some()
        );
        assert!(
            routes
                .wrote(Method::DELETE, "/realms/acme/clients/c1")
                .is_none()
        );
    }

    #[tokio::test]
    async fn removing_what_is_not_there_writes_nothing() {
        let world = World::new(vec![
            (
                Method::GET,
                "/realms/acme/clients",
                json!({ "data": [gateway()] }),
            ),
            (
                Method::GET,
                "/realms/acme/clients/c1/token-exchange-policies",
                json!([]),
            ),
            (
                Method::GET,
                "/realms/acme/resource-owners",
                json!({ "data": [] }),
            ),
        ]);

        remove_server(&world, "acme", "crm", CRM_URI)
            .await
            .expect("nothing to do");

        assert_eq!(world.writes(), Vec::new());
    }
}
