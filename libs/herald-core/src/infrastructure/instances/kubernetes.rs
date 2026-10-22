use autharie_crds::v1alpha::identity_instance::{IdentityInstance, Settled};
use kube::{Api, Client, ResourceExt, api::ListParams};
use uuid::Uuid;

use crate::domain::{
    entities::deployment::DeploymentId,
    error::HeraldError,
    outcome_reconciliation::{InstanceState, ObservedInstances},
    ports::InstanceStateSource,
};

const DEPLOYMENT_INSTANCE_PREFIX: &str = "deployment-";

pub struct KubeInstanceStateSource {
    client: Client,
}

impl KubeInstanceStateSource {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn from_env() -> Result<Self, HeraldError> {
        let client = Client::try_default()
            .await
            .map_err(|error| HeraldError::Internal {
                message: format!("failed to build a Kubernetes client: {error}"),
            })?;

        Ok(Self::new(client))
    }
}

impl InstanceStateSource for KubeInstanceStateSource {
    async fn observe(&self) -> Result<ObservedInstances, HeraldError> {
        let instances: Api<IdentityInstance> = Api::all(self.client.clone());

        let listed = instances
            .list(&ListParams::default())
            .await
            .map_err(|error| HeraldError::Internal {
                message: format!("the instances on this cluster could not be listed: {error}"),
            })?;

        Ok(listed.items.iter().filter_map(observed).collect())
    }

    async fn is_absent(
        &self,
        deployment_id: &DeploymentId,
        namespace: &str,
    ) -> Result<bool, HeraldError> {
        let instances: Api<IdentityInstance> = Api::namespaced(self.client.clone(), namespace);

        let found = instances
            .get_opt(&instance_name(deployment_id))
            .await
            .map_err(|error| HeraldError::Internal {
                message: format!(
                    "the instance of deployment {deployment_id} could not be read: {error}"
                ),
            })?;

        Ok(found.is_none())
    }
}

fn instance_name(deployment_id: &DeploymentId) -> String {
    format!("{DEPLOYMENT_INSTANCE_PREFIX}{deployment_id}")
}

fn observed(instance: &IdentityInstance) -> Option<(DeploymentId, InstanceState)> {
    let deployment_id = deployment_id_of(&instance.name_any())?;
    let state = match instance.status.as_ref().and_then(|status| status.settled()) {
        Some(Settled::Serving) => InstanceState::Serving {
            version: instance.spec.version.clone(),
        },
        Some(Settled::Failed) => InstanceState::Failed,
        None => InstanceState::Unsettled,
    };

    Some((deployment_id, state))
}

pub(crate) fn deployment_id_of(name: &str) -> Option<DeploymentId> {
    let id = Uuid::parse_str(name.strip_prefix(DEPLOYMENT_INSTANCE_PREFIX)?).ok()?;
    Some(DeploymentId::new(id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_crds::common::types::Phase;
    use autharie_crds::v1alpha::identity_instance::{IdentityInstanceSpec, IdentityInstanceStatus};

    fn instance(name: &str, status: Option<IdentityInstanceStatus>) -> IdentityInstance {
        let spec: IdentityInstanceSpec = serde_json::from_value(serde_json::json!({
            "organisationId": "org-1",
            "provider": "ferriskey",
            "version": "26.0.1",
            "hostname": "acme.example.com",
            "database": {
                "managedCluster": {
                    "storage": { "size": "1Gi" },
                    "resources": {}
                }
            }
        }))
        .expect("a spec");
        let mut instance = IdentityInstance::new(name, spec);
        instance.status = status;
        instance
    }

    fn status(phase: Phase, ready: bool) -> Option<IdentityInstanceStatus> {
        Some(IdentityInstanceStatus {
            phase: Some(phase),
            ready,
            ..Default::default()
        })
    }

    #[test]
    fn a_serving_instance_carries_the_version_it_was_told_to_run() {
        let id = Uuid::new_v4();

        let observation = observed(&instance(
            &format!("deployment-{id}"),
            status(Phase::Running, true),
        ));

        assert_eq!(
            observation,
            Some((
                DeploymentId::new(id.to_string()),
                InstanceState::Serving {
                    version: "26.0.1".to_string()
                }
            ))
        );
    }

    #[test]
    fn an_instance_on_its_way_is_unsettled() {
        let id = Uuid::new_v4();

        for status in [
            None,
            status(Phase::Running, false),
            status(Phase::Deleting, true),
        ] {
            let observation = observed(&instance(&format!("deployment-{id}"), status));

            assert_eq!(
                observation.map(|(_, state)| state),
                Some(InstanceState::Unsettled)
            );
        }
    }

    #[test]
    fn a_failed_instance_is_failed() {
        let observation = observed(&instance(
            &format!("deployment-{}", Uuid::new_v4()),
            status(Phase::Failed, false),
        ));

        assert_eq!(
            observation.map(|(_, state)| state),
            Some(InstanceState::Failed)
        );
    }

    #[test]
    fn the_instance_looked_for_is_the_one_genesis_names_after_the_deployment() {
        let id = Uuid::new_v4();
        let name = instance_name(&DeploymentId::new(id.to_string()));

        assert_eq!(name, format!("deployment-{id}"));
        assert_eq!(
            deployment_id_of(&name),
            Some(DeploymentId::new(id.to_string()))
        );
    }

    #[test]
    fn an_instance_no_deployment_owns_is_not_observed() {
        for name in ["cloud-iam-ferriskey", "drill-1", "deployment-not-a-uuid"] {
            assert_eq!(
                observed(&instance(name, status(Phase::Running, true))),
                None,
                "{name}"
            );
        }
    }
}
