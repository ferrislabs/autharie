use autharie_crds::v1alpha::identity_instance::{AgentPolicySpec, AgentsSpec, IdentityInstance};
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;
use tracing::info;

use crate::domain::entities::agents_payload::{AgentPolicyDefinition, AgentsPayloadV1};
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::GenesisError;
use crate::domain::ports::AgentsPort;

pub const AGENTS_FIELD_MANAGER: &str = "genesis-agents";

pub struct KubeAgentsPort {
    client: Client,
}

impl KubeAgentsPort {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn instance(
        &self,
        reference: &IdentityInstanceRef,
    ) -> Result<IdentityInstance, GenesisError> {
        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), &reference.namespace);

        api.get_opt(&reference.name)
            .await
            .map_err(|error| kubernetes_error("read the IdentityInstance", &error))?
            .ok_or_else(|| GenesisError::Kubernetes {
                message: format!("IdentityInstance {} does not exist yet", reference.name),
            })
    }
}

impl AgentsPort for KubeAgentsPort {
    async fn revision<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
    ) -> Result<u64, GenesisError> {
        Ok(self
            .instance(reference)
            .await?
            .spec
            .agents
            .map_or(0, |agents| agents.policies_revision))
    }

    async fn apply<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        desired: &'a AgentsPayloadV1,
    ) -> Result<(), GenesisError> {
        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), &reference.namespace);

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            "setting the agent settings"
        );

        api.patch(
            &reference.name,
            &PatchParams {
                field_manager: Some(AGENTS_FIELD_MANAGER.to_string()),
                ..PatchParams::default()
            },
            &Patch::Merge(&agents_patch(desired)),
        )
        .await
        .map_err(|error| kubernetes_error("set the agent settings", &error))?;

        Ok(())
    }
}

fn kubernetes_error(doing: &str, error: &kube::Error) -> GenesisError {
    let detail = match error {
        kube::Error::Api(response) => format!("{} {}", response.code, response.reason),
        _ => "transport error".to_string(),
    };

    GenesisError::Kubernetes {
        message: format!("failed to {doing}: {detail}"),
    }
}

fn to_policy(definition: &AgentPolicyDefinition) -> AgentPolicySpec {
    AgentPolicySpec {
        id: definition.id.to_string(),
        name: definition.name.clone(),
        cedar: definition.cedar.clone(),
    }
}

pub fn to_spec(desired: &AgentsPayloadV1) -> AgentsSpec {
    AgentsSpec {
        enabled: desired.enabled,
        realm: desired.realm.clone(),
        policies: desired.policies.iter().map(to_policy).collect(),
        policies_revision: desired.revision,
    }
}

pub fn agents_patch(desired: &AgentsPayloadV1) -> serde_json::Value {
    let spec = to_spec(desired);

    json!({
        "spec": {
            "agents": {
                "enabled": spec.enabled,
                "realm": spec.realm,
                "policies": spec.policies,
                "policiesRevision": spec.policies_revision,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const POLICY: Uuid = Uuid::from_u128(2);

    fn desired(enabled: bool, policies: Value) -> AgentsPayloadV1 {
        AgentsPayloadV1::from_value(&json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "enabled": enabled,
            "realm": "acme",
            "revision": 7,
            "policies": policies,
        }))
        .expect("valid")
    }

    #[test]
    fn the_patch_carries_the_spec_fields_in_camel_case() {
        let patch = agents_patch(&desired(
            true,
            json!([{ "id": POLICY, "name": "Read only", "cedar": "permit (principal, action, resource);" }]),
        ));

        assert_eq!(
            patch,
            json!({
                "spec": { "agents": {
                    "enabled": true,
                    "realm": "acme",
                    "policies": [{
                        "id": POLICY.to_string(),
                        "name": "Read only",
                        "cedar": "permit (principal, action, resource);",
                    }],
                    "policiesRevision": 7,
                } }
            })
        );
    }

    #[test]
    fn an_emptied_policy_list_is_named_so_the_old_one_is_cleared() {
        let patch = agents_patch(&desired(false, json!([])));

        assert_eq!(patch["spec"]["agents"]["policies"], json!([]));
        assert_eq!(patch["spec"]["agents"]["enabled"], json!(false));
    }

    #[test]
    fn the_patch_touches_nothing_but_the_agents_field() {
        let patch = agents_patch(&desired(true, json!([])));

        let spec = patch["spec"].as_object().expect("object");
        assert_eq!(spec.keys().collect::<Vec<_>>(), vec!["agents"]);
    }

    #[test]
    fn the_spec_converts_every_policy_with_its_id_as_text() {
        let spec = to_spec(&desired(
            true,
            json!([{ "id": POLICY, "name": "n", "cedar": "c" }]),
        ));

        assert_eq!(spec.policies[0].id, POLICY.to_string());
        assert_eq!(spec.policies_revision, 7);
    }

    #[test]
    fn a_cluster_error_names_the_step_and_never_the_request() {
        let error = kubernetes_error(
            "set the agent settings",
            &kube::Error::Api(kube::core::ErrorResponse {
                status: "Failure".to_string(),
                message: "invalid value permit (principal == secret)".to_string(),
                reason: "Invalid".to_string(),
                code: 422,
            }),
        );

        assert!(error.to_string().contains("422"));
        assert!(!error.to_string().contains("secret"));
    }
}
