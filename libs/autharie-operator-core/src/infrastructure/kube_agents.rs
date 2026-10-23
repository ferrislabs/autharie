use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, DynamicObject, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client, Resource};
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::{AgentsStatus, IdentityInstance};

use crate::domain::OperatorError;
use crate::domain::agents::ports::{
    AgentInstances, AgentsEdge, AgentsSecretsPort, AgentsStatusRepository, GatewayConfigStore,
};
use crate::domain::agents::{AgentsError, GATEWAY_CONFIG_KEY, GATEWAY_CONFIG_SECRET, servers};

use super::edge::{Backend, Edge, build_hostname_route, exposed, httproute_api_resource};
use super::ferriskey_admin::name_and_namespace;

const CLIENT_SECRET_KEY: &str = "client_secret";
const CLIENT_ID_KEY: &str = "client_id";
const GATEWAY_FIELD_MANAGER: &str = "autharie-operator";

pub const ROUTE_LABEL: &str = "autharie.fr/agents-route";
pub const ROUTE_INSTANCE_LABEL: &str = "autharie.fr/instance";
pub const ROUTE_NAMESPACE_LABEL: &str = "autharie.fr/instance-namespace";

pub fn client_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-mcp-gateway-client")
}

pub fn route_name(instance_name: &str) -> String {
    format!("mcp-{instance_name}")
}

fn located(instance: &IdentityInstance) -> Result<(&str, &str), AgentsError> {
    name_and_namespace(instance).ok_or_else(|| AgentsError::Retry {
        message: "the instance has no name or namespace yet".to_string(),
    })
}

fn is_not_found(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(response) if response.code == 404)
}

fn kube_error(doing: &str, error: kube::Error) -> AgentsError {
    let reason = match &error {
        kube::Error::Api(response) => format!("status {}", response.code),
        _ => "no answer".to_string(),
    };
    AgentsError::Retry {
        message: format!("the cluster could not {doing} ({reason})"),
    }
}

async fn read_key(
    secrets: &Api<Secret>,
    name: &str,
    key: &str,
) -> Result<Option<String>, AgentsError> {
    let secret = secrets
        .get_opt(name)
        .await
        .map_err(|error| kube_error("read an agents secret", error))?;
    Ok(secret
        .and_then(|secret| secret.data)
        .and_then(|mut data| data.remove(key))
        .and_then(|value| String::from_utf8(value.0).ok()))
}

pub struct KubeAgentsSecrets {
    client: Client,
}

impl KubeAgentsSecrets {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl AgentsSecretsPort for KubeAgentsSecrets {
    async fn load(&self, instance: &IdentityInstance) -> Result<Option<String>, AgentsError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        read_key(&secrets, &client_secret_name(name), CLIENT_SECRET_KEY).await
    }

    async fn store(&self, instance: &IdentityInstance, secret: &str) -> Result<(), AgentsError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let manifest = Secret {
            metadata: ObjectMeta {
                name: Some(client_secret_name(name)),
                namespace: Some(namespace.to_string()),
                owner_references: instance.controller_owner_ref(&()).map(|owner| vec![owner]),
                ..Default::default()
            },
            string_data: Some(BTreeMap::from([
                (
                    CLIENT_ID_KEY.to_string(),
                    crate::domain::agents::GATEWAY_CLIENT_ID.to_string(),
                ),
                (CLIENT_SECRET_KEY.to_string(), secret.to_string()),
            ])),
            ..Default::default()
        };

        secrets
            .patch(
                &client_secret_name(name),
                &PatchParams::apply(GATEWAY_FIELD_MANAGER).force(),
                &Patch::Apply(&manifest),
            )
            .await
            .map(|_| ())
            .map_err(|error| kube_error("write the gateway client secret", error))
    }

    async fn forget(&self, instance: &IdentityInstance) -> Result<(), AgentsError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        match secrets
            .delete(&client_secret_name(name), &DeleteParams::default())
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(kube_error("delete the gateway client secret", error)),
        }
    }
}

pub fn agents_status_patch(status: &AgentsStatus) -> Value {
    json!({
        "status": {
            "agents": {
                "phase": status.phase,
                "gatewayClientReady": status.gateway_client_ready,
                "observedRevision": status.observed_revision,
                "mcpUrl": status.mcp_url,
                "message": status.message,
                "policies": status.policies,
                "servers": status.servers,
            }
        }
    })
}

pub struct KubeAgentsStatus {
    client: Client,
}

impl KubeAgentsStatus {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl AgentsStatusRepository for KubeAgentsStatus {
    async fn patch_agents_status(
        &self,
        instance: &IdentityInstance,
        status: &AgentsStatus,
    ) -> Result<(), OperatorError> {
        let name = instance
            .metadata
            .name
            .as_deref()
            .ok_or(OperatorError::MissingName)?;
        let namespace = instance.metadata.namespace.as_deref().ok_or_else(|| {
            OperatorError::MissingNamespace {
                name: name.to_string(),
            }
        })?;

        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), namespace);
        api.patch_status(
            name,
            &PatchParams::default(),
            &Patch::Merge(&agents_status_patch(status)),
        )
        .await
        .map(|_| ())
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })
    }
}

pub struct KubeAgentInstances {
    client: Client,
}

impl KubeAgentInstances {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl AgentInstances for KubeAgentInstances {
    async fn list(&self) -> Result<Vec<IdentityInstance>, AgentsError> {
        let api: Api<IdentityInstance> = Api::all(self.client.clone());
        api.list(&ListParams::default())
            .await
            .map(|list| list.items)
            .map_err(|error| kube_error("list the instances", error))
    }
}

pub struct KubeGatewayConfig {
    client: Client,
    namespace: String,
}

impl KubeGatewayConfig {
    pub fn new(client: Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }
}

impl GatewayConfigStore for KubeGatewayConfig {
    async fn current(&self) -> Result<Option<String>, AgentsError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);
        read_key(&secrets, GATEWAY_CONFIG_SECRET, GATEWAY_CONFIG_KEY).await
    }

    async fn publish(&self, document: &str) -> Result<(), AgentsError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);
        let patch = json!({ "stringData": { GATEWAY_CONFIG_KEY: document } });

        match secrets
            .patch(
                GATEWAY_CONFIG_SECRET,
                &PatchParams::default(),
                &Patch::Merge(&patch),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => {
                let secret = Secret {
                    metadata: ObjectMeta {
                        name: Some(GATEWAY_CONFIG_SECRET.to_string()),
                        namespace: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    string_data: Some(BTreeMap::from([(
                        GATEWAY_CONFIG_KEY.to_string(),
                        document.to_string(),
                    )])),
                    ..Default::default()
                };
                secrets
                    .create(&PostParams::default(), &secret)
                    .await
                    .map(|_| ())
                    .map_err(|error| kube_error("create the gateway configuration", error))
            }
            Err(error) => Err(kube_error("write the gateway configuration", error)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayService {
    pub name: String,
    pub port: i32,
}

impl GatewayService {
    pub const DEFAULT_NAME: &'static str = "mcp-gateway";
    pub const DEFAULT_PORT: i32 = 8080;

    pub fn from_env() -> Self {
        let read = |key: &str| {
            std::env::var(key)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self {
            name: read("AUTHARIE_MCP_GATEWAY_SERVICE")
                .unwrap_or_else(|| Self::DEFAULT_NAME.to_string()),
            port: read("AUTHARIE_MCP_GATEWAY_PORT")
                .and_then(|value| value.parse().ok())
                .unwrap_or(Self::DEFAULT_PORT),
        }
    }
}

pub const MCP_PATHS: [&str; 2] = ["/mcp", "/.well-known/oauth-protected-resource/mcp"];

pub fn server_paths(slug: &str) -> [String; 2] {
    MCP_PATHS.map(|path| format!("{path}/{slug}"))
}

pub fn build_mcp_route(
    instance: &IdentityInstance,
    edge: &Edge,
    service: &GatewayService,
) -> Value {
    let name = instance.metadata.name.clone().unwrap_or_default();
    let namespace = instance.metadata.namespace.clone().unwrap_or_default();
    let labels = BTreeMap::from([
        (ROUTE_LABEL.to_string(), "true".to_string()),
        (ROUTE_INSTANCE_LABEL.to_string(), name.clone()),
        (ROUTE_NAMESPACE_LABEL.to_string(), namespace),
    ]);

    let customer_paths = instance
        .spec
        .agents
        .as_ref()
        .map(servers::serving)
        .unwrap_or_default()
        .into_iter()
        .filter(|server| servers::is_slug(&server.slug))
        .flat_map(|server| server_paths(&server.slug));
    let backends: Vec<Backend> = MCP_PATHS
        .iter()
        .map(|path| path.to_string())
        .chain(customer_paths)
        .map(|path| Backend::new(path, service.name.clone(), service.port))
        .collect();

    build_hostname_route(
        &route_name(&name),
        &edge.gateway_namespace,
        &labels,
        edge,
        std::slice::from_ref(&instance.spec.hostname),
        &backends,
    )
}

pub struct KubeAgentsEdge {
    client: Client,
    edge: Edge,
    service: GatewayService,
}

impl KubeAgentsEdge {
    pub fn new(client: Client, edge: Edge, service: GatewayService) -> Self {
        Self {
            client,
            edge,
            service,
        }
    }

    fn routes(&self) -> Api<DynamicObject> {
        Api::namespaced_with(
            self.client.clone(),
            &self.edge.gateway_namespace,
            &httproute_api_resource(),
        )
    }
}

impl AgentsEdge for KubeAgentsEdge {
    async fn apply_route(&self, instance: &IdentityInstance) -> Result<(), AgentsError> {
        let (name, _) = located(instance)?;
        if !exposed(instance) {
            return self.remove_route(instance).await;
        }

        let route = build_mcp_route(instance, &self.edge, &self.service);
        self.routes()
            .patch(
                &route_name(name),
                &PatchParams::apply(GATEWAY_FIELD_MANAGER).force(),
                &Patch::Apply(&route),
            )
            .await
            .map(|_| ())
            .map_err(|error| kube_error("apply the MCP route", error))
    }

    async fn remove_route(&self, instance: &IdentityInstance) -> Result<(), AgentsError> {
        let (name, namespace) = located(instance)?;
        let routes = self.routes();
        let route_name = route_name(name);

        let found = routes
            .get_opt(&route_name)
            .await
            .map_err(|error| kube_error("look for the MCP route", error))?;
        let Some(found) = found else {
            return Ok(());
        };
        let labels = found.metadata.labels.unwrap_or_default();
        let ours = labels.get(ROUTE_LABEL).map(String::as_str) == Some("true")
            && labels.get(ROUTE_INSTANCE_LABEL).map(String::as_str) == Some(name)
            && labels.get(ROUTE_NAMESPACE_LABEL).map(String::as_str) == Some(namespace);
        if !ours {
            return Ok(());
        }

        match routes.delete(&route_name, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(kube_error("delete the MCP route", error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use autharie_crds::v1alpha::identity_instance::{
        AgentPolicyPhase, AgentPolicyStatus, AgentsPhase, AgentsSpec,
    };

    use super::*;
    use crate::domain::agents::test_spec;

    fn instance() -> IdentityInstance {
        let mut instance = IdentityInstance::new("deployment-1", test_spec("auth.acme.test"));
        instance.metadata.namespace = Some("tenant-a".to_string());
        instance.spec.agents = Some(AgentsSpec {
            enabled: true,
            realm: "acme".to_string(),
            policies: Vec::new(),
            policies_revision: 0,
            servers: Vec::new(),
        });
        instance
    }

    fn edge() -> Edge {
        Edge::new("autharie-dataplane", "autharie-system")
    }

    fn service() -> GatewayService {
        GatewayService {
            name: "mcp-gateway".to_string(),
            port: 8080,
        }
    }

    #[test]
    fn the_names_follow_the_instance() {
        assert_eq!(
            client_secret_name("deployment-1"),
            "deployment-1-mcp-gateway-client"
        );
        assert_eq!(route_name("deployment-1"), "mcp-deployment-1");
    }

    #[test]
    fn the_route_serves_the_mcp_endpoint_and_its_metadata_to_the_gateway() {
        let route = build_mcp_route(&instance(), &edge(), &service());

        assert_eq!(route["kind"], "HTTPRoute");
        assert_eq!(route["metadata"]["name"], "mcp-deployment-1");
        assert_eq!(route["metadata"]["namespace"], "autharie-system");
        assert_eq!(route["spec"]["hostnames"], json!(["auth.acme.test"]));
        let rules = route["spec"]["rules"].as_array().expect("rules");
        let paths: Vec<_> = rules
            .iter()
            .flat_map(|rule| rule["matches"].as_array().expect("matches"))
            .map(|m| (m["path"]["type"].clone(), m["path"]["value"].clone()))
            .collect();
        assert_eq!(
            paths,
            vec![
                (json!("PathPrefix"), json!("/mcp")),
                (
                    json!("PathPrefix"),
                    json!("/.well-known/oauth-protected-resource/mcp")
                ),
            ]
        );
        for rule in rules {
            assert_eq!(
                rule["backendRefs"],
                json!([{ "name": "mcp-gateway", "port": 8080 }])
            );
        }
    }

    fn with_customers(servers: serde_json::Value) -> IdentityInstance {
        let mut found = instance();
        found.spec.agents.as_mut().expect("agents").servers =
            serde_json::from_value(servers).expect("servers");
        found
    }

    fn paths_of(route: &Value) -> Vec<String> {
        route["spec"]["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .flat_map(|rule| rule["matches"].as_array().expect("matches"))
            .map(|m| m["path"]["value"].as_str().expect("a path").to_string())
            .collect()
    }

    #[test]
    fn spec_ms_2_each_customer_server_adds_its_own_paths_to_the_route() {
        let route = build_mcp_route(
            &with_customers(json!([
                {
                    "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
                    "slug": "crm",
                    "url": "https://tools.example.com/mcp",
                    "enabled": true,
                },
                {
                    "id": "7f1c2d3e-0000-4000-8000-00000000000c",
                    "slug": "erp",
                    "url": "https://erp.example.com/mcp",
                    "enabled": false,
                },
                {
                    "id": "7f1c2d3e-0000-4000-8000-00000000000d",
                    "slug": "gateway",
                    "url": "https://x.example.com/mcp",
                    "enabled": true,
                },
            ])),
            &edge(),
            &service(),
        );

        assert_eq!(
            paths_of(&route),
            vec![
                "/mcp",
                "/.well-known/oauth-protected-resource/mcp",
                "/mcp/crm",
                "/.well-known/oauth-protected-resource/mcp/crm",
            ]
        );
        for rule in route["spec"]["rules"].as_array().expect("rules") {
            assert_eq!(
                rule["backendRefs"],
                json!([{ "name": "mcp-gateway", "port": 8080 }])
            );
        }
    }

    #[test]
    fn spec_ms_7_a_removed_server_leaves_the_route_with_the_paths_that_remain() {
        let route = build_mcp_route(&instance(), &edge(), &service());

        assert_eq!(
            paths_of(&route),
            vec!["/mcp", "/.well-known/oauth-protected-resource/mcp"]
        );
    }

    #[test]
    fn the_route_attaches_to_the_data_plane_gateway() {
        let route = build_mcp_route(&instance(), &edge(), &service());

        assert_eq!(
            route["spec"]["parentRefs"],
            json!([{
                "group": "gateway.networking.k8s.io",
                "kind": "Gateway",
                "name": "autharie-dataplane",
                "namespace": "autharie-system",
            }])
        );
    }

    #[test]
    fn the_route_says_whose_it_is_since_no_owner_can_say_it_across_namespaces() {
        let route = build_mcp_route(&instance(), &edge(), &service());

        assert_eq!(route["metadata"]["labels"][ROUTE_LABEL], "true");
        assert_eq!(
            route["metadata"]["labels"][ROUTE_INSTANCE_LABEL],
            "deployment-1"
        );
        assert_eq!(
            route["metadata"]["labels"][ROUTE_NAMESPACE_LABEL],
            "tenant-a"
        );
        assert!(route["metadata"].get("ownerReferences").is_none());
    }

    #[test]
    fn the_gateway_service_defaults_when_nothing_is_set() {
        assert_eq!(GatewayService::DEFAULT_NAME, "mcp-gateway");
        assert_eq!(GatewayService::DEFAULT_PORT, 8080);
    }

    #[test]
    fn the_status_patch_names_every_field_so_an_old_value_is_cleared() {
        let patch = agents_status_patch(&crate::domain::agents::settled_off(4));

        let agents = patch["status"]["agents"].as_object().expect("object");
        assert_eq!(agents["phase"], "Ready");
        assert_eq!(agents["gatewayClientReady"], false);
        assert_eq!(agents["observedRevision"], 4);
        assert_eq!(agents["mcpUrl"], Value::Null);
        assert_eq!(agents["message"], Value::Null);
        assert_eq!(agents["policies"], json!([]));
        assert_eq!(agents["servers"], json!([]));
        assert_eq!(patch["status"].as_object().expect("object").len(), 1);
    }

    #[test]
    fn the_status_patch_carries_each_server_with_its_tools() {
        use autharie_crds::v1alpha::identity_instance::{
            AgentServerPhase, AgentServerStatus, AgentServerTool,
        };

        let patch = agents_status_patch(&AgentsStatus {
            servers: vec![AgentServerStatus {
                id: "s1".to_string(),
                slug: "crm".to_string(),
                phase: AgentServerPhase::Ready,
                tools: vec![AgentServerTool {
                    name: "list_invoices".to_string(),
                    description: None,
                    read_only: true,
                    destructive: false,
                }],
                dropped: 1,
                message: None,
                discovered_at: Some("2026-10-23T10:00:00.000Z".to_string()),
                discovery_requested_at: Some("2026-10-23T09:59:59.000Z".to_string()),
            }],
            ..crate::domain::agents::settled_off(4)
        });

        assert_eq!(
            patch["status"]["agents"]["servers"],
            json!([{
                "id": "s1",
                "slug": "crm",
                "phase": "Ready",
                "tools": [{ "name": "list_invoices", "readOnly": true, "destructive": false }],
                "dropped": 1,
                "discoveredAt": "2026-10-23T10:00:00.000Z",
                "discoveryRequestedAt": "2026-10-23T09:59:59.000Z",
            }])
        );
    }

    #[test]
    fn the_status_patch_carries_each_policy() {
        let patch = agents_status_patch(&AgentsStatus {
            phase: AgentsPhase::Applying,
            gateway_client_ready: true,
            observed_revision: 2,
            mcp_url: Some("https://auth.acme.test/mcp".to_string()),
            message: Some("waiting".to_string()),
            policies: vec![AgentPolicyStatus {
                id: "p1".to_string(),
                phase: AgentPolicyPhase::Invalid,
                message: Some("unexpected token".to_string()),
            }],
            servers: Vec::new(),
        });

        assert_eq!(patch["status"]["agents"]["phase"], "Applying");
        assert_eq!(
            patch["status"]["agents"]["policies"][0],
            json!({ "id": "p1", "phase": "Invalid", "message": "unexpected token" })
        );
        assert_eq!(
            patch["status"]["agents"]["mcpUrl"],
            "https://auth.acme.test/mcp"
        );
    }

    #[test]
    fn a_cluster_error_never_quotes_the_request() {
        let error = kube_error(
            "write the gateway client secret",
            kube::Error::Api(kube::core::ErrorResponse {
                status: "Failure".to_string(),
                message: "invalid value s3cret-value".to_string(),
                reason: "Invalid".to_string(),
                code: 422,
            }),
        );

        assert!(!error.message().contains("s3cret"));
        assert!(error.message().contains("422"));
    }
}
