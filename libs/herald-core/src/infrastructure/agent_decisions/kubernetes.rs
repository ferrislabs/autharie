use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::IdentityInstance;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use tokio::sync::mpsc;

use crate::domain::agent_decisions::Tenant;
use crate::domain::entities::logs::{LogLine, OrganisationId};
use crate::domain::error::HeraldError;
use crate::domain::ports::{GatewayLogSource, TenantSource};
use crate::infrastructure::instances::kubernetes::deployment_id_of;
use crate::infrastructure::logs::kubernetes::follow_container;

const GATEWAY_SELECTOR: &str = "app.kubernetes.io/component=mcp-gateway";
const BUFFER: usize = 1_024;
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const SINCE_SECONDS: i64 = 6 * 60;

pub struct KubeGatewayLogSource {
    client: Client,
    namespace: String,
}

impl KubeGatewayLogSource {
    pub fn new(client: Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    pub async fn from_env(namespace: impl Into<String>) -> Result<Self, HeraldError> {
        Ok(Self::new(client_from_env().await?, namespace))
    }
}

async fn client_from_env() -> Result<Client, HeraldError> {
    Client::try_default()
        .await
        .map_err(|error| HeraldError::Internal {
            message: format!("failed to build a Kubernetes client: {error}"),
        })
}

impl GatewayLogSource for KubeGatewayLogSource {
    async fn follow(&self) -> Result<mpsc::Receiver<LogLine>, HeraldError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let params = ListParams::default().labels(GATEWAY_SELECTOR);

        let listed = tokio::time::timeout(LIST_TIMEOUT, pods.list(&params))
            .await
            .map_err(|_| HeraldError::Internal {
                message: "listing the MCP gateway pods timed out".to_string(),
            })?
            .map_err(|error| HeraldError::Internal {
                message: format!("could not list the MCP gateway pods: {error}"),
            })?;

        let containers: Vec<(String, String)> = listed
            .items
            .iter()
            .filter_map(|pod| {
                let name = pod.metadata.name.clone()?;
                let containers = pod.spec.as_ref()?.containers.iter();
                Some(
                    containers
                        .map(|container| (name.clone(), container.name.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect();

        if containers.is_empty() {
            return Err(HeraldError::Internal {
                message: format!("no pods matching {GATEWAY_SELECTOR} in {}", self.namespace),
            });
        }

        let (sender, receiver) = mpsc::channel(BUFFER);
        for (pod, container) in containers {
            tokio::spawn(follow_container(
                pods.clone(),
                pod,
                container,
                SINCE_SECONDS,
                sender.clone(),
            ));
        }

        Ok(receiver)
    }
}

pub struct KubeTenantSource {
    client: Client,
}

impl KubeTenantSource {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn from_env() -> Result<Self, HeraldError> {
        Ok(Self::new(client_from_env().await?))
    }
}

impl TenantSource for KubeTenantSource {
    async fn tenants(&self) -> Result<Vec<(String, Tenant)>, HeraldError> {
        let instances: Api<IdentityInstance> = Api::all(self.client.clone());
        let listed = instances
            .list(&ListParams::default())
            .await
            .map_err(|error| HeraldError::Internal {
                message: format!("the instances on this cluster could not be listed: {error}"),
            })?;

        Ok(listed.items.iter().filter_map(tenant_of).collect())
    }
}

fn tenant_of(instance: &IdentityInstance) -> Option<(String, Tenant)> {
    let deployment_id = deployment_id_of(&instance.name_any())?;
    Some((
        instance.spec.hostname.clone(),
        Tenant {
            organisation_id: OrganisationId::new(instance.spec.organisation_id.clone()),
            deployment_id,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_crds::v1alpha::identity_instance::IdentityInstanceSpec;

    fn instance(name: &str) -> IdentityInstance {
        let spec: IdentityInstanceSpec = serde_json::from_value(serde_json::json!({
            "organisationId": "org-1",
            "provider": "ferriskey",
            "version": "26.0.1",
            "hostname": "auth.acme.com",
            "database": {
                "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} }
            }
        }))
        .expect("a spec");
        IdentityInstance::new(name, spec)
    }

    #[test]
    fn an_instance_maps_its_hostname_to_its_organisation_and_deployment() {
        let id = "11111111-1111-1111-1111-111111111111";
        let (host, tenant) = tenant_of(&instance(&format!("deployment-{id}"))).expect("a tenant");

        assert_eq!(host, "auth.acme.com");
        assert_eq!(tenant.organisation_id, OrganisationId::new("org-1"));
        assert_eq!(tenant.deployment_id.to_string(), id);
    }

    #[test]
    fn an_instance_not_named_after_a_deployment_is_skipped() {
        assert!(tenant_of(&instance("scratch")).is_none());
    }
}
