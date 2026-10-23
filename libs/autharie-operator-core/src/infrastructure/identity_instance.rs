use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use autharie_crds::common::types::Phase;
use autharie_crds::v1alpha::identity_instance::{
    DatabaseMode, IdentityInstance, IdentityProvider, ManagedClusterConfig,
};
use autharie_crds::v1alpha::identity_instance_upgrade::IdentityInstanceUpgrade;
use futures::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, DeploymentStrategy};
use k8s_openapi::api::batch::v1::{CronJob, Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Affinity, Container, ContainerPort, EnvVar, EnvVarSource, HTTPGetAction, PodAffinityTerm,
    PodAntiAffinity, PodSpec, PodTemplateSpec, Probe, Secret, SecretKeySelector, Service,
    ServicePort, ServiceSpec, WeightedPodAffinityTerm,
};
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::events::{Event as KubeEvent, EventType, Recorder, Reporter};
use kube::runtime::watcher;
use kube::{Api, Client, Resource};
use rand::{Rng, distributions::Alphanumeric};
use serde_json::{Value, json};
use tracing::{error, info, warn};

use crate::application::OperatorApplication;
use crate::domain::agents::authz_required;
use crate::domain::agents::config::AgentsEnvironment;
use crate::domain::agents::ports::{AgentsOutcome, IdentityInstanceAgents};
use crate::domain::agents::service::AgentsReconciler;
use crate::domain::ports::{
    IdentityInstanceDeployer, IdentityInstanceRepository, IdentityInstanceService,
};
use crate::domain::shared_database::SharedDatabaseName;
use crate::domain::webhooks::ports::{IdentityInstanceWebhooks, WebhookOutcome};
use crate::domain::webhooks::service::WebhookReconciler;
use crate::domain::{OperatorError, ReconcileOutcome};
use crate::infrastructure::archive::{
    ArchiveStore, cluster_backup_section, cluster_recovery_section,
};
use crate::infrastructure::dumps::{DumpTarget, build_dump_cronjob, cluster_image, cronjob_name};
use crate::infrastructure::edge::{
    Backend, Edge, allowed_ranges, build_route, build_security_policy, exposed,
    httproute_api_resource, route_is_ready, security_policy_api_resource,
};
use crate::infrastructure::ferriskey_agents::{KubeAgentServers, KubeFerriskeyAgents};
use crate::infrastructure::ferriskey_theme::KubeInstanceTheme;
use crate::infrastructure::ferriskey_webhooks::KubeFerriskeyWebhooks;
use crate::infrastructure::kube_agents::{
    GatewayService, KubeAgentInstances, KubeAgentsEdge, KubeAgentsSecrets, KubeAgentsStatus,
    KubeGatewayConfig,
};
use crate::infrastructure::kube_webhooks::{
    KubeWebhookDeliveries, KubeWebhookSecrets, KubeWebhookStatus,
};
use crate::infrastructure::shared_database::{self, SharedCluster};

pub struct KubeIdentityInstanceRepository {
    client: Client,
}

impl KubeIdentityInstanceRepository {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

/// A merge patch leaves a field it does not mention as it was, so a field of
/// `iam` that is now empty has to be named as null or the old value stays.
fn status_merge_patch(
    status: &autharie_crds::v1alpha::identity_instance::IdentityInstanceStatus,
) -> Value {
    let mut patch = json!({ "status": status });
    // A merge patch leaves an absent field as it was, so a status that has
    // recovered must say so explicitly: without the null, the error of the
    // last failed reconcile stays next to `ready: true` for ever.
    if let Some(top) = patch["status"].as_object_mut() {
        top.entry("error").or_insert(Value::Null);
        top.entry("conditions").or_insert_with(|| json!([]));
    }
    if let Some(iam) = patch["status"]
        .get_mut("iam")
        .and_then(Value::as_object_mut)
    {
        for field in ["message", "applied", "observedAt"] {
            iam.entry(field).or_insert(Value::Null);
        }
    }
    patch
}

impl IdentityInstanceRepository for KubeIdentityInstanceRepository {
    async fn patch_status(
        &self,
        instance: &IdentityInstance,
        status: autharie_crds::v1alpha::identity_instance::IdentityInstanceStatus,
    ) -> Result<IdentityInstance, OperatorError> {
        let previous_status = instance.status.clone().unwrap_or_default();
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;

        let namespace = instance
            .metadata
            .namespace
            .clone()
            .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;

        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), &namespace);
        let patch = status_merge_patch(&status);

        let updated = api
            .patch_status(
                &name,
                &kube::api::PatchParams::default(),
                &kube::api::Patch::Merge(&patch),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        if let Err(error) = self
            .publish_status_event(instance, &previous_status, &status)
            .await
        {
            warn!(
                name = %name,
                namespace = %namespace,
                error = %error,
                "Failed to publish IdentityInstance status event"
            );
        }

        Ok(updated)
    }
}

impl KubeIdentityInstanceRepository {
    async fn publish_status_event(
        &self,
        instance: &IdentityInstance,
        previous: &autharie_crds::v1alpha::identity_instance::IdentityInstanceStatus,
        current: &autharie_crds::v1alpha::identity_instance::IdentityInstanceStatus,
    ) -> Result<(), OperatorError> {
        let previous_phase = previous
            .phase
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "None".to_string());
        let current_phase = current
            .phase
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "None".to_string());

        let reporter = Reporter {
            controller: "autharie-operator".to_string(),
            instance: Some("identityinstance-controller".to_string()),
        };
        let recorder = Recorder::new(self.client.clone(), reporter);
        let reference = instance.object_ref(&());
        let phase_note = match current_phase.as_str() {
            "DatabaseProvisioning" => "Database cluster provisioning is in progress.",
            "Deploying" => "Database is ready. Deploying identity provider and ingress resources.",
            "Running" => "Identity provider is healthy and ready.",
            "Stopped" => {
                "Instance is stopped: workloads are scaled to zero and the database is hibernated."
            }
            _ => "IdentityInstance status updated.",
        };

        let event = KubeEvent {
            type_: EventType::Normal,
            reason: "StatusUpdated".to_string(),
            note: Some(format!(
                "{phase_note} Transition: phase {previous_phase} -> {current_phase}, ready {} -> {}",
                previous.ready, current.ready,
            )),
            action: "StatusPatch".to_string(),
            secondary: None,
        };

        recorder
            .publish(&event, &reference)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(())
    }
}

pub struct KubeIdentityInstanceDeployer {
    client: Client,
    handlers: Vec<Arc<dyn IdentityProviderHandler>>,
}

impl KubeIdentityInstanceDeployer {
    /// `otlp_endpoint` is where this cluster's own Herald accepts OTLP
    /// traces, when this installation was told to ship any at all --
    /// `None` is every installation that has not turned tracing on, and
    /// every FerrisKey instance this deployer provisions is left with
    /// observability off, the same "absent means off" shape [`Edge`]
    /// does not need since a Gateway is never optional the way tracing is.
    ///
    /// `public_https_port` is only ever `Some` on a cluster whose Gateway is
    /// not actually reachable on 443 -- a local k3d cluster mapping it to a
    /// host port instead, since a container cannot bind the real one. A
    /// browser sent to `https://{hostname}` with nothing to say otherwise
    /// assumes 443, and a redirect landing there instead of the mapped port
    /// resolves the hostname (it is in `/etc/hosts`) but cannot reach
    /// anything listening. Every other Gateway -- staging, production, or a
    /// dedicated cluster with its own real ingress -- terminates 443 as
    /// 443, and this stays `None`.
    pub fn new(
        client: Client,
        edge: Edge,
        otlp_endpoint: Option<String>,
        public_https_port: Option<u16>,
        shared_cluster: Option<SharedCluster>,
    ) -> Self {
        let handlers: Vec<Arc<dyn IdentityProviderHandler>> = vec![
            Arc::new(KeycloakProviderHandler::new(client.clone(), edge.clone())),
            Arc::new(FerriskeyProviderHandler::new(
                client.clone(),
                edge,
                otlp_endpoint,
                public_https_port,
                shared_cluster,
            )),
        ];
        Self { client, handlers }
    }

    fn handler_for(
        &self,
        provider: &IdentityProvider,
    ) -> Option<&Arc<dyn IdentityProviderHandler>> {
        self.handlers
            .iter()
            .find(|handler| handler.provider() == *provider)
    }
}

impl IdentityInstanceDeployer for KubeIdentityInstanceDeployer {
    async fn ensure_provider_resources(
        &self,
        instance: &IdentityInstance,
    ) -> Result<(), OperatorError> {
        let provider = &instance.spec.provider;
        let handler = self
            .handler_for(provider)
            .ok_or_else(|| OperatorError::Internal {
                message: format!("no deployer handler registered for provider `{provider}`"),
            })?;
        handler.ensure(instance).await
    }

    async fn cleanup_provider_resources(
        &self,
        instance: &IdentityInstance,
    ) -> Result<(), OperatorError> {
        let provider = &instance.spec.provider;
        let handler = self
            .handler_for(provider)
            .ok_or_else(|| OperatorError::Internal {
                message: format!("no deployer handler registered for provider `{provider}`"),
            })?;
        handler.cleanup(instance).await
    }

    async fn provider_ready(&self, instance: &IdentityInstance) -> Result<bool, OperatorError> {
        let provider = &instance.spec.provider;
        let handler = self
            .handler_for(provider)
            .ok_or_else(|| OperatorError::Internal {
                message: format!("no deployer handler registered for provider `{provider}`"),
            })?;
        handler.ready(instance).await
    }

    async fn database_ready(&self, instance: &IdentityInstance) -> Result<bool, OperatorError> {
        let provider = &instance.spec.provider;
        let handler = self
            .handler_for(provider)
            .ok_or_else(|| OperatorError::Internal {
                message: format!("no deployer handler registered for provider `{provider}`"),
            })?;
        handler.database_ready(instance).await
    }

    async fn edge_ready(&self, instance: &IdentityInstance) -> Result<bool, OperatorError> {
        let provider = &instance.spec.provider;
        let handler = self
            .handler_for(provider)
            .ok_or_else(|| OperatorError::Internal {
                message: format!("no deployer handler registered for provider `{provider}`"),
            })?;
        handler.edge_ready(instance).await
    }

    async fn upgrade_in_progress(
        &self,
        instance: &IdentityInstance,
    ) -> Result<bool, OperatorError> {
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let namespace = instance
            .metadata
            .namespace
            .clone()
            .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;
        let upgrades: Api<IdentityInstanceUpgrade> =
            Api::namespaced(self.client.clone(), &namespace);
        let list = upgrades
            .list(&kube::api::ListParams::default())
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(list.items.iter().any(|upgrade| {
            upgrade.spec.identity_instance_ref.name == name
                && upgrade.spec.approved
                && !upgrade_is_over(upgrade)
        }))
    }
}

/// Whether an upgrade has stopped moving, either way.
///
/// `Running` is the one that succeeded. `Failed` is the one that gave up, and
/// it has to count too: treating it as still in progress is what kept an
/// instance re-asserting `Upgrading` every fifteen seconds after the deadline
/// had already declared the upgrade dead.
fn upgrade_is_over(upgrade: &IdentityInstanceUpgrade) -> bool {
    matches!(
        upgrade
            .status
            .as_ref()
            .and_then(|status| status.phase.clone()),
        Some(Phase::Running) | Some(Phase::Failed)
    )
}

type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<(), OperatorError>> + Send + 'a>>;
type ProviderReadyFuture<'a> =
    Pin<Box<dyn Future<Output = Result<bool, OperatorError>> + Send + 'a>>;

trait IdentityProviderHandler: Send + Sync {
    fn provider(&self) -> IdentityProvider;
    fn ensure<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderFuture<'a>;
    fn cleanup<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderFuture<'a>;
    fn ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a>;
    fn database_ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a>;
    fn edge_ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a>;
}

struct KeycloakProviderHandler {
    client: Client,
    edge: Edge,
}

impl KeycloakProviderHandler {
    fn new(client: Client, edge: Edge) -> Self {
        Self { client, edge }
    }

    async fn ensure_keycloak_admin_secret(
        &self,
        namespace: &str,
        secret_name: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        if let Some(existing) =
            secrets
                .get_opt(secret_name)
                .await
                .map_err(|error| OperatorError::Kube {
                    message: error.to_string(),
                })?
            && let Some(data) = existing.data.as_ref()
            && data.contains_key("username")
            && data.contains_key("password")
        {
            return Ok(());
        }

        let password = generate_password(32);
        let mut string_data = BTreeMap::new();
        string_data.insert("username".to_string(), "admin".to_string());
        string_data.insert("password".to_string(), password);

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(secret_name.to_string()),
                namespace: Some(namespace.to_string()),
                owner_references: owner_reference.map(|owner| vec![owner]),
                ..Default::default()
            },
            type_: Some("Opaque".to_string()),
            string_data: Some(string_data),
            ..Default::default()
        };

        secrets
            .patch(
                secret_name,
                &kube::api::PatchParams::apply("autharie-operator").force(),
                &kube::api::Patch::Apply(&secret),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(())
    }

    async fn ensure_keycloak_db_credentials_secret(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<bool, OperatorError> {
        let instance_name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let target_secret_name = keycloak_db_credentials_secret_name(&instance_name);
        let source_secret_name = format!("{}-app", cnpg_cluster_name(instance));
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        if let Some(existing) = secrets
            .get_opt(&target_secret_name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?
            && let Some(data) = existing.data.as_ref()
            && data.contains_key("jdbc-uri")
            && data.contains_key("user")
            && data.contains_key("password")
        {
            return Ok(true);
        }

        let source = secrets
            .get_opt(&source_secret_name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        let Some(source) = source else {
            return Ok(false);
        };

        let data = source.data.ok_or_else(|| OperatorError::Internal {
            message: format!("CNPG secret `{}` has no data", source_secret_name),
        })?;

        let username = secret_data_value(&data, "username")
            .or_else(|| secret_data_value(&data, "user"))
            .ok_or_else(|| OperatorError::Internal {
                message: format!("CNPG secret `{}` missing `username`", source_secret_name),
            })?;
        let password =
            secret_data_value(&data, "password").ok_or_else(|| OperatorError::Internal {
                message: format!("CNPG secret `{}` missing `password`", source_secret_name),
            })?;
        let jdbc_uri = secret_data_value(&data, "jdbc-uri")
            .or_else(|| {
                // Fallback for CNPG variants that provide only `uri`.
                secret_data_value(&data, "uri").map(|uri| {
                    if uri.starts_with("jdbc:") {
                        uri
                    } else if let Some(rest) = uri.strip_prefix("postgresql://") {
                        format!("jdbc:postgresql://{rest}")
                    } else if let Some(rest) = uri.strip_prefix("postgres://") {
                        format!("jdbc:postgresql://{rest}")
                    } else {
                        uri
                    }
                })
            })
            .ok_or_else(|| OperatorError::Internal {
                message: format!(
                    "CNPG secret `{}` missing `jdbc-uri` and `uri`",
                    source_secret_name
                ),
            })?;

        let mut string_data = BTreeMap::new();
        string_data.insert("user".to_string(), username);
        string_data.insert("password".to_string(), password);
        string_data.insert("jdbc-uri".to_string(), jdbc_uri);

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(target_secret_name.clone()),
                namespace: Some(namespace.to_string()),
                owner_references: owner_reference.map(|owner| vec![owner]),
                ..Default::default()
            },
            type_: Some("Opaque".to_string()),
            string_data: Some(string_data),
            ..Default::default()
        };

        secrets
            .patch(
                &target_secret_name,
                &kube::api::PatchParams::apply("autharie-operator").force(),
                &kube::api::Patch::Apply(&secret),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(true)
    }

    async fn keycloak_ready(&self, instance: &IdentityInstance) -> Result<bool, OperatorError> {
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let namespace = instance
            .metadata
            .namespace
            .clone()
            .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &namespace);

        let deployment = deployments
            .get_opt(&name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        let Some(deployment) = deployment else {
            return Ok(false);
        };

        Ok(replicas_ready(&deployment, desired_replicas(instance)))
    }

    async fn may_hibernate(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
    ) -> Result<bool, OperatorError> {
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let deployments = existing_deployments(&self.client, namespace, &[name]).await?;
        Ok(hibernation_due(
            instance,
            &deployments,
            &[],
            chrono::Utc::now(),
        ))
    }

    async fn apply_keycloak_workload(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        admin_secret_name: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let labels = keycloak_labels(instance);
        let deployment = build_keycloak_deployment(
            instance,
            &name,
            namespace,
            &labels,
            admin_secret_name,
            owner_reference.clone(),
        )?;
        let service =
            build_keycloak_service(instance, &name, namespace, &labels, owner_reference.clone())?;

        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        let services: Api<Service> = Api::namespaced(self.client.clone(), namespace);

        let params = kube::api::PatchParams::apply("autharie-operator").force();
        deployments
            .patch(&name, &params, &kube::api::Patch::Apply(&deployment))
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        services
            .patch(&name, &params, &kube::api::Patch::Apply(&service))
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        self.ensure_keycloak_route(instance, namespace, owner_reference)
            .await
    }

    async fn ensure_keycloak_route(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        if !exposed(instance) {
            return Ok(());
        }
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let labels = keycloak_labels(instance);
        let backends = [Backend::new("/", &name, 80)];

        apply_route(
            self.client.clone(),
            instance,
            &name,
            namespace,
            &labels,
            owner_reference,
            &self.edge,
            &backends,
        )
        .await
    }

    async fn keycloak_route_ready(
        &self,
        instance: &IdentityInstance,
    ) -> Result<bool, OperatorError> {
        route_ready_or_unexposed(self.client.clone(), instance).await
    }

    async fn cnpg_cluster_ready(&self, instance: &IdentityInstance) -> Result<bool, OperatorError> {
        let namespace =
            instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace {
                    name: instance.metadata.name.clone().unwrap_or_default(),
                })?;
        let cluster_name = cnpg_cluster_name(instance);
        let gvk = GroupVersionKind::gvk("postgresql.cnpg.io", "v1", "Cluster");
        let ar = ApiResource::from_gvk(&gvk);
        let clusters: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), &namespace, &ar);

        let cluster =
            clusters
                .get_opt(&cluster_name)
                .await
                .map_err(|error| OperatorError::Kube {
                    message: error.to_string(),
                })?;

        let Some(cluster) = cluster else {
            return Ok(false);
        };

        let status = cluster.data.get("status");
        let Some(conditions) = status
            .and_then(|status| status.get("conditions"))
            .and_then(|conditions| conditions.as_array())
        else {
            return Ok(false);
        };

        let ready = conditions.iter().any(|condition| {
            condition.get("type").and_then(|value| value.as_str()) == Some("Ready")
                && condition.get("status").and_then(|value| value.as_str()) == Some("True")
        });

        Ok(ready)
    }
}

impl IdentityProviderHandler for KeycloakProviderHandler {
    fn provider(&self) -> IdentityProvider {
        IdentityProvider::Keycloak
    }

    fn ensure<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderFuture<'a> {
        Box::pin(async move {
            let name = instance
                .metadata
                .name
                .clone()
                .ok_or(OperatorError::MissingName)?;
            let namespace = instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;

            info!(
                name = %name,
                namespace = %namespace,
                provider = "keycloak",
                "Ensuring provider resources"
            );

            let admin_secret_name = keycloak_admin_secret_name(&name);
            let owner_reference = instance.controller_owner_ref(&());
            if instance.spec.stopped {
                self.apply_keycloak_workload(
                    instance,
                    &namespace,
                    &admin_secret_name,
                    owner_reference.clone(),
                )
                .await?;
                let hibernate = self.may_hibernate(instance, &namespace).await?;
                return apply_managed_db_cluster(
                    &self.client,
                    instance,
                    &namespace,
                    owner_reference,
                    hibernate,
                )
                .await;
            }
            apply_managed_db_cluster(
                &self.client,
                instance,
                &namespace,
                owner_reference.clone(),
                false,
            )
            .await?;
            let db_cluster_ready = self.cnpg_cluster_ready(instance).await?;
            if !db_cluster_ready {
                info!(
                    name = %name,
                    namespace = %namespace,
                    provider = "keycloak",
                    "Waiting for CNPG cluster readiness before deploying provider resources"
                );
                return Ok(());
            }
            let db_secret_ready = self
                .ensure_keycloak_db_credentials_secret(
                    instance,
                    &namespace,
                    owner_reference.clone(),
                )
                .await?;
            if !db_secret_ready {
                info!(
                    name = %name,
                    namespace = %namespace,
                    provider = "keycloak",
                    "Waiting for CNPG credentials secret before deploying provider resources"
                );
                return Ok(());
            }
            self.ensure_keycloak_admin_secret(
                &namespace,
                &admin_secret_name,
                owner_reference.clone(),
            )
            .await?;
            self.apply_keycloak_workload(instance, &namespace, &admin_secret_name, owner_reference)
                .await?;

            info!(
                name = %name,
                namespace = %namespace,
                provider = "keycloak",
                "Provider resources applied"
            );

            Ok(())
        })
    }

    fn cleanup<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderFuture<'a> {
        Box::pin(async move {
            let name = instance
                .metadata
                .name
                .clone()
                .ok_or(OperatorError::MissingName)?;
            let namespace = instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;
            let delete_params = kube::api::DeleteParams::default();

            info!(
                name = %name,
                namespace = %namespace,
                provider = "keycloak",
                "Cleaning up provider resources"
            );

            let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &namespace);
            if let Err(error) = deployments.delete(&name, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }

            let services: Api<Service> = Api::namespaced(self.client.clone(), &namespace);
            if let Err(error) = services.delete(&name, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }
            let ingresses: Api<Ingress> = Api::namespaced(self.client.clone(), &namespace);
            if let Err(error) = ingresses.delete(&name, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }
            remove_edge(self.client.clone(), &name, &namespace).await?;

            let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &namespace);
            let admin_secret = keycloak_admin_secret_name(&name);
            if let Err(error) = secrets.delete(&admin_secret, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }

            Ok(())
        })
    }

    fn ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a> {
        Box::pin(async move { self.keycloak_ready(instance).await })
    }

    fn database_ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a> {
        Box::pin(async move { self.cnpg_cluster_ready(instance).await })
    }

    fn edge_ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a> {
        Box::pin(async move { self.keycloak_route_ready(instance).await })
    }
}

struct FerriskeyProviderHandler {
    client: Client,
    edge: Edge,
    /// Passed straight through to every instance's `OTLP_ENDPOINT` (and
    /// `METRICS_ENDPOINT` -- see `build_ferriskey_api_deployment`'s own
    /// comment on why both are needed) -- see
    /// [`KubeIdentityInstanceDeployer::new`]'s own comment on why `None` is
    /// a legitimate, common state rather than a misconfiguration.
    otlp_endpoint: Option<String>,
    /// See [`KubeIdentityInstanceDeployer::new`]'s own comment on this field.
    public_https_port: Option<u16>,
    /// Where this data plane's free deployments keep their databases, from
    /// the operator's own environment. An instance in shared mode may name
    /// another cluster in its spec; this is what it falls back to.
    shared_cluster: Option<SharedCluster>,
}

impl FerriskeyProviderHandler {
    fn new(
        client: Client,
        edge: Edge,
        otlp_endpoint: Option<String>,
        public_https_port: Option<u16>,
        shared_cluster: Option<SharedCluster>,
    ) -> Self {
        Self {
            client,
            edge,
            otlp_endpoint,
            public_https_port,
            shared_cluster,
        }
    }

    fn shared_cluster_of(
        &self,
        instance: &IdentityInstance,
    ) -> Result<Option<SharedCluster>, OperatorError> {
        if instance.spec.database.mode != DatabaseMode::Shared {
            return Ok(None);
        }
        Ok(Some(SharedCluster::resolve(
            instance.spec.database.shared.as_ref(),
            self.shared_cluster.as_ref(),
        )?))
    }

    async fn ensure_shared_database(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
        cluster: &SharedCluster,
    ) -> Result<(), OperatorError> {
        let instance_name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let name = SharedDatabaseName::for_instance_name(&instance_name)?;
        let target_secret_name = ferriskey_db_credentials_secret_name(&instance_name);
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        let existing = secrets
            .get_opt(&target_secret_name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        let kept = existing
            .as_ref()
            .and_then(|secret| secret.data.as_ref())
            .and_then(|data| secret_data_value(data, "password"));
        let password = kept.unwrap_or_else(|| generate_password(32));

        let pool = shared_database::connect(&self.client, cluster).await?;
        let provisioned = shared_database::provision(&pool, &name, &password).await;
        pool.close().await;
        provisioned?;

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(target_secret_name.clone()),
                namespace: Some(namespace.to_string()),
                owner_references: owner_reference.map(|owner| vec![owner]),
                ..Default::default()
            },
            type_: Some("Opaque".to_string()),
            string_data: Some(shared_database::tenant_secret_data(
                cluster, &name, &password,
            )),
            ..Default::default()
        };

        secrets
            .patch(
                &target_secret_name,
                &kube::api::PatchParams::apply("autharie-operator").force(),
                &kube::api::Patch::Apply(&secret),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(())
    }

    async fn drop_shared_database(
        &self,
        instance: &IdentityInstance,
        cluster: &SharedCluster,
    ) -> Result<(), OperatorError> {
        let instance_name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let name = SharedDatabaseName::for_instance_name(&instance_name)?;

        let pool = shared_database::connect(&self.client, cluster).await?;
        let dropped = shared_database::drop_database(&pool, &name).await;
        pool.close().await;
        Ok(dropped?)
    }

    async fn shared_database_ready(
        &self,
        instance: &IdentityInstance,
    ) -> Result<bool, OperatorError> {
        let namespace =
            instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace {
                    name: instance.metadata.name.clone().unwrap_or_default(),
                })?;
        let secret_name = ferriskey_db_credentials_secret_name(
            &instance
                .metadata
                .name
                .clone()
                .ok_or(OperatorError::MissingName)?,
        );
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &namespace);
        let secret = secrets
            .get_opt(&secret_name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(secret
            .and_then(|secret| secret.data)
            .is_some_and(|data| data.contains_key("database-url")))
    }

    async fn cnpg_cluster_ready(&self, instance: &IdentityInstance) -> Result<bool, OperatorError> {
        let namespace =
            instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace {
                    name: instance.metadata.name.clone().unwrap_or_default(),
                })?;
        let cluster_name = cnpg_cluster_name(instance);
        let gvk = GroupVersionKind::gvk("postgresql.cnpg.io", "v1", "Cluster");
        let ar = ApiResource::from_gvk(&gvk);
        let clusters: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), &namespace, &ar);

        let cluster =
            clusters
                .get_opt(&cluster_name)
                .await
                .map_err(|error| OperatorError::Kube {
                    message: error.to_string(),
                })?;

        let Some(cluster) = cluster else {
            return Ok(false);
        };

        let status = cluster.data.get("status");
        let Some(conditions) = status
            .and_then(|status| status.get("conditions"))
            .and_then(|conditions| conditions.as_array())
        else {
            return Ok(false);
        };

        Ok(conditions.iter().any(|condition| {
            condition.get("type").and_then(|value| value.as_str()) == Some("Ready")
                && condition.get("status").and_then(|value| value.as_str()) == Some("True")
        }))
    }

    async fn ensure_ferriskey_db_credentials_secret(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<bool, OperatorError> {
        let instance_name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let source_secret_name = format!("{}-app", cnpg_cluster_name(instance));
        let target_secret_name = ferriskey_db_credentials_secret_name(&instance_name);
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        if let Some(existing) = secrets
            .get_opt(&target_secret_name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?
            && let Some(data) = existing.data.as_ref()
            && data.contains_key("database-url")
            && data.contains_key("user")
            && data.contains_key("password")
        {
            return Ok(true);
        }

        let source = secrets
            .get_opt(&source_secret_name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        let Some(source) = source else {
            return Ok(false);
        };
        let data = source.data.ok_or_else(|| OperatorError::Internal {
            message: format!("CNPG secret `{}` has no data", source_secret_name),
        })?;

        let username = secret_data_value(&data, "username")
            .or_else(|| secret_data_value(&data, "user"))
            .ok_or_else(|| OperatorError::Internal {
                message: format!("CNPG secret `{}` missing `username`", source_secret_name),
            })?;
        let password =
            secret_data_value(&data, "password").ok_or_else(|| OperatorError::Internal {
                message: format!("CNPG secret `{}` missing `password`", source_secret_name),
            })?;
        let database_url =
            secret_data_value(&data, "uri").ok_or_else(|| OperatorError::Internal {
                message: format!("CNPG secret `{}` missing `uri`", source_secret_name),
            })?;

        let mut string_data = BTreeMap::new();
        string_data.insert("database-url".to_string(), database_url);
        string_data.insert("user".to_string(), username);
        string_data.insert("password".to_string(), password);

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(target_secret_name.clone()),
                namespace: Some(namespace.to_string()),
                owner_references: owner_reference.map(|owner| vec![owner]),
                ..Default::default()
            },
            type_: Some("Opaque".to_string()),
            string_data: Some(string_data),
            ..Default::default()
        };

        secrets
            .patch(
                &target_secret_name,
                &kube::api::PatchParams::apply("autharie-operator").force(),
                &kube::api::Patch::Apply(&secret),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(true)
    }

    async fn ensure_ferriskey_admin_secret(
        &self,
        namespace: &str,
        secret_name: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        if let Some(existing) =
            secrets
                .get_opt(secret_name)
                .await
                .map_err(|error| OperatorError::Kube {
                    message: error.to_string(),
                })?
            && let Some(data) = existing.data.as_ref()
            && data.contains_key("username")
            && data.contains_key("password")
        {
            return Ok(());
        }

        let password = generate_password(32);
        let mut string_data = BTreeMap::new();
        string_data.insert("username".to_string(), "admin".to_string());
        string_data.insert("password".to_string(), password);
        string_data.insert("email".to_string(), "admin@cluster.local".to_string());

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(secret_name.to_string()),
                namespace: Some(namespace.to_string()),
                owner_references: owner_reference.map(|owner| vec![owner]),
                ..Default::default()
            },
            type_: Some("Opaque".to_string()),
            string_data: Some(string_data),
            ..Default::default()
        };

        secrets
            .patch(
                secret_name,
                &kube::api::PatchParams::apply("autharie-operator").force(),
                &kube::api::Patch::Apply(&secret),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(())
    }

    /// A Job that ended Failed is never retried by Kubernetes, and one that failed because the
    /// database was taken from under it would leave the instance waiting for ever. Only on the
    /// restart: a migration that fails on a running instance stays visibly Failed.
    async fn clear_failed_migration(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
    ) -> Result<(), OperatorError> {
        if !restarting(instance) {
            return Ok(());
        }
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
        let name = ferriskey_migration_job_name(instance);
        let existing = jobs
            .get_opt(&name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        if !existing.as_ref().is_some_and(migration_failed) {
            return Ok(());
        }

        info!(job = %name, "deleting a failed migration so the restart runs it again");
        match jobs
            .delete(
                &name,
                &kube::api::DeleteParams {
                    propagation_policy: Some(kube::api::PropagationPolicy::Background),
                    ..Default::default()
                },
            )
            .await
        {
            Err(error) if !is_not_found(&error) => Err(OperatorError::Kube {
                message: error.to_string(),
            }),
            _ => Ok(()),
        }
    }

    async fn ensure_ferriskey_migration_job(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
        let name = ferriskey_migration_job_name(instance);
        if jobs
            .get_opt(&name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?
            .is_some()
        {
            return Ok(());
        }

        let db_secret_name = ferriskey_db_credentials_secret_name(
            &instance
                .metadata
                .name
                .clone()
                .ok_or(OperatorError::MissingName)?,
        );
        let image = ferriskey_api_image(instance);
        let labels = ferriskey_labels(instance, "migrations");
        let job = build_ferriskey_migration_job(
            &name,
            namespace,
            &labels,
            &image,
            &db_secret_name,
            owner_reference,
        )?;

        jobs.create(&kube::api::PostParams::default(), &job)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(())
    }

    async fn ferriskey_migration_completed(
        &self,
        instance: &IdentityInstance,
    ) -> Result<bool, OperatorError> {
        let namespace =
            instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace {
                    name: instance.metadata.name.clone().unwrap_or_default(),
                })?;
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &namespace);
        let name = ferriskey_migration_job_name(instance);
        let job = jobs
            .get_opt(&name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        let Some(job) = job else {
            return Ok(false);
        };
        Ok(job
            .status
            .as_ref()
            .and_then(|status| status.succeeded)
            .unwrap_or(0)
            > 0)
    }

    async fn ensure_ferriskey_runtime_resources(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        let shared = self.shared_cluster_of(instance)?;
        let runtime = build_ferriskey_runtime(
            instance,
            namespace,
            owner_reference,
            self.otlp_endpoint.as_deref(),
            self.public_https_port,
            shared.as_ref(),
        )?;
        let FerriskeyRuntime {
            api: api_deployment,
            api_service,
            webapp: web_deployment,
            webapp_service: web_service,
        } = runtime;
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let api_name = ferriskey_api_name(&name);
        let web_name = ferriskey_webapp_name(&name);

        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        let services: Api<Service> = Api::namespaced(self.client.clone(), namespace);
        let params = kube::api::PatchParams::apply("autharie-operator").force();

        if is_recreated(&api_deployment) {
            clear_rolling_update(&deployments, &api_name).await?;
        }
        deployments
            .patch(
                &api_name,
                &params,
                &kube::api::Patch::Apply(&api_deployment),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        services
            .patch(&api_name, &params, &kube::api::Patch::Apply(&api_service))
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        deployments
            .patch(
                &web_name,
                &params,
                &kube::api::Patch::Apply(&web_deployment),
            )
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        services
            .patch(&web_name, &params, &kube::api::Patch::Apply(&web_service))
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;

        Ok(())
    }

    /// The daily dump of a shared database, or nothing when the instance names
    /// no destination.
    ///
    /// In shared mode `spec.backup` is read as where the dumps go; no
    /// CloudNativePG backup object is ever made from it, because there is no
    /// cluster of the tenant's to back up. An instance whose destination is
    /// taken away has its CronJob removed, so a tenant that stopped being
    /// dumped does not keep being dumped on the strength of an old apply.
    async fn ensure_shared_dumps(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let jobs: Api<CronJob> = Api::namespaced(self.client.clone(), namespace);
        let job_name = cronjob_name(&name);

        let Some(backup) = instance.spec.backup.as_ref() else {
            match jobs
                .delete(&job_name, &kube::api::DeleteParams::default())
                .await
            {
                Ok(_) => {}
                Err(kube::Error::Api(response)) if response.code == 404 => {}
                Err(error) => {
                    return Err(OperatorError::Kube {
                        message: error.to_string(),
                    });
                }
            }
            return Ok(());
        };

        let Some(store) = ArchiveStore::from_env() else {
            return Err(OperatorError::Configuration {
                message: format!(
                    "instance `{name}` is dumped to {} and this data plane has no object store credentials",
                    backup.destination_path
                ),
            });
        };
        ensure_archive_credentials(&self.client, instance, namespace).await?;

        // Asked of the cluster, not written down: see `DumpTarget`. An error
        // rather than a quiet return, because the error is what requeues; a
        // quiet return from a finished reconcile would leave the tenant
        // undumped until something else happened to touch it.
        let cluster =
            self.shared_cluster_of(instance)?
                .ok_or_else(|| OperatorError::Configuration {
                    message: format!("instance `{name}` is not in shared mode"),
                })?;
        let clusters: Api<DynamicObject> = Api::namespaced_with(
            self.client.clone(),
            &cluster.namespace,
            &ApiResource::from_gvk(&GroupVersionKind::gvk(
                "postgresql.cnpg.io",
                "v1",
                "Cluster",
            )),
        );
        let shared =
            clusters
                .get_opt(&cluster.name)
                .await
                .map_err(|error| OperatorError::Kube {
                    message: error.to_string(),
                })?;
        let Some(postgres_image) = shared
            .as_ref()
            .and_then(|shared| cluster_image(&shared.data))
        else {
            return Err(OperatorError::Internal {
                message: format!(
                    "the shared cluster `{}` does not report its image yet, so `{name}` has no dump job",
                    cluster.name
                ),
            });
        };

        let owner = owner_reference
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| OperatorError::Internal {
                message: error.to_string(),
            })?;
        let cronjob = build_dump_cronjob(
            &DumpTarget {
                instance_name: &name,
                namespace,
                database_secret: &ferriskey_db_credentials_secret_name(&name),
                backup,
                region: &store.region,
                postgres_image,
            },
            owner,
        )?;

        jobs.patch(
            &job_name,
            &kube::api::PatchParams::apply("autharie-operator").force(),
            &kube::api::Patch::Apply(&cronjob),
        )
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;

        Ok(())
    }

    /// Workloads first; the database follows once none is left and no migration
    /// runs. Migrations, the credentials copy and shared-database provisioning
    /// wait for the start.
    async fn ensure_stopped(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        self.ensure_ferriskey_runtime_resources(instance, namespace, owner_reference.clone())
            .await?;
        self.ensure_ferriskey_route(instance, namespace, owner_reference.clone())
            .await?;
        if self.shared_cluster_of(instance)?.is_some() {
            return self
                .ensure_shared_dumps(instance, namespace, owner_reference)
                .await;
        }
        let hibernate = self.may_hibernate(instance, namespace).await?;
        apply_managed_db_cluster(
            &self.client,
            instance,
            namespace,
            owner_reference,
            hibernate,
        )
        .await
    }

    async fn migration_jobs(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
    ) -> Result<Vec<Job>, OperatorError> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
        let listed = jobs
            .list(&kube::api::ListParams::default().labels(&ferriskey_migration_selector(instance)))
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        Ok(listed.items)
    }

    async fn may_hibernate(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
    ) -> Result<bool, OperatorError> {
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let deployments = existing_deployments(
            &self.client,
            namespace,
            &[ferriskey_api_name(&name), ferriskey_webapp_name(&name)],
        )
        .await?;
        let jobs = self.migration_jobs(instance, namespace).await?;
        Ok(hibernation_due(
            instance,
            &deployments,
            &jobs,
            chrono::Utc::now(),
        ))
    }

    async fn ensure_ferriskey_route(
        &self,
        instance: &IdentityInstance,
        namespace: &str,
        owner_reference: Option<OwnerReference>,
    ) -> Result<(), OperatorError> {
        if !exposed(instance) {
            return Ok(());
        }
        let name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let labels = ferriskey_labels(instance, "route");
        let backends = [
            Backend::new("/api", ferriskey_api_name(&name), FERRISKEY_API_PORT),
            Backend::new("/", ferriskey_webapp_name(&name), 80),
        ];

        apply_route(
            self.client.clone(),
            instance,
            &name,
            namespace,
            &labels,
            owner_reference,
            &self.edge,
            &backends,
        )
        .await
    }

    async fn ferriskey_route_ready(
        &self,
        instance: &IdentityInstance,
    ) -> Result<bool, OperatorError> {
        route_ready_or_unexposed(self.client.clone(), instance).await
    }

    async fn ferriskey_runtime_ready(
        &self,
        instance: &IdentityInstance,
    ) -> Result<bool, OperatorError> {
        let namespace =
            instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace {
                    name: instance.metadata.name.clone().unwrap_or_default(),
                })?;
        let instance_name = instance
            .metadata
            .name
            .clone()
            .ok_or(OperatorError::MissingName)?;
        let api_name = ferriskey_api_name(&instance_name);
        let web_name = ferriskey_webapp_name(&instance_name);
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &namespace);
        let api_ready =
            deployment_ready(&deployments, &api_name, desired_replicas(instance)).await?;
        let web_ready =
            deployment_ready(&deployments, &web_name, desired_replicas(instance)).await?;
        if instance.spec.stopped {
            let jobs = self.migration_jobs(instance, &namespace).await?;
            let now = chrono::Utc::now();
            return Ok(api_ready
                && web_ready
                && !jobs.iter().any(|job| migration_holds_hibernation(job, now)));
        }
        let migration_done = self.ferriskey_migration_completed(instance).await?;

        Ok(api_ready && web_ready && migration_done)
    }
}

impl IdentityProviderHandler for FerriskeyProviderHandler {
    fn provider(&self) -> IdentityProvider {
        IdentityProvider::Ferriskey
    }

    fn ensure<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderFuture<'a> {
        Box::pin(async move {
            let name = instance
                .metadata
                .name
                .clone()
                .ok_or(OperatorError::MissingName)?;
            let namespace = instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;
            let owner_reference = instance.controller_owner_ref(&());
            if instance.spec.stopped {
                return self
                    .ensure_stopped(instance, &namespace, owner_reference)
                    .await;
            }
            self.clear_failed_migration(instance, &namespace).await?;
            if let Some(cluster) = self.shared_cluster_of(instance)? {
                self.ensure_shared_database(
                    instance,
                    &namespace,
                    owner_reference.clone(),
                    &cluster,
                )
                .await?;
            } else {
                apply_managed_db_cluster(
                    &self.client,
                    instance,
                    &namespace,
                    owner_reference.clone(),
                    false,
                )
                .await?;
                let db_cluster_ready = self.cnpg_cluster_ready(instance).await?;
                if !db_cluster_ready {
                    info!(
                        name = %name,
                        namespace = %namespace,
                        provider = "ferriskey",
                        "Waiting for CNPG cluster readiness before deploying provider resources"
                    );
                    return Ok(());
                }
                let db_secret_ready = self
                    .ensure_ferriskey_db_credentials_secret(
                        instance,
                        &namespace,
                        owner_reference.clone(),
                    )
                    .await?;
                if !db_secret_ready {
                    info!(
                        name = %name,
                        namespace = %namespace,
                        provider = "ferriskey",
                        "Waiting for CNPG credentials secret before running Ferriskey migrations"
                    );
                    return Ok(());
                }
            }

            self.ensure_ferriskey_admin_secret(
                &namespace,
                &ferriskey_api_admin_secret_name(&name),
                owner_reference.clone(),
            )
            .await?;

            self.ensure_ferriskey_migration_job(instance, &namespace, owner_reference.clone())
                .await?;
            if !self.ferriskey_migration_completed(instance).await? {
                info!(
                    name = %name,
                    namespace = %namespace,
                    provider = "ferriskey",
                    "Waiting for Ferriskey database migrations to complete"
                );
                return Ok(());
            }

            self.ensure_ferriskey_runtime_resources(instance, &namespace, owner_reference)
                .await?;
            self.ensure_ferriskey_route(instance, &namespace, instance.controller_owner_ref(&()))
                .await?;
            // Last, so a destination that cannot be reached costs a tenant its
            // dumps and not its sign-in page.
            if instance.spec.database.mode == DatabaseMode::Shared {
                self.ensure_shared_dumps(instance, &namespace, instance.controller_owner_ref(&()))
                    .await?;
            }

            info!(
                name = %name,
                namespace = %namespace,
                provider = "ferriskey",
                "Ferriskey resources applied"
            );
            Ok(())
        })
    }

    fn cleanup<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderFuture<'a> {
        Box::pin(async move {
            let name = instance
                .metadata
                .name
                .clone()
                .ok_or(OperatorError::MissingName)?;
            let namespace = instance
                .metadata
                .namespace
                .clone()
                .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;
            let delete_params = kube::api::DeleteParams::default();
            let shared = self.shared_cluster_of(instance)?;
            let cluster_name = cnpg_cluster_name(instance);
            let api_name = ferriskey_api_name(&name);
            let web_name = ferriskey_webapp_name(&name);
            // Every version's job, not only the one this instance currently
            // runs: there is one per version now, and tearing down must not
            // leave the ones an upgrade left behind.
            let migration_jobs = ferriskey_migration_selector(instance);
            let db_secret_name = ferriskey_db_credentials_secret_name(&name);
            let gvk = GroupVersionKind::gvk("postgresql.cnpg.io", "v1", "Cluster");
            let ar = ApiResource::from_gvk(&gvk);
            let clusters: Api<DynamicObject> =
                Api::namespaced_with(self.client.clone(), &namespace, &ar);
            let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &namespace);
            let services: Api<Service> = Api::namespaced(self.client.clone(), &namespace);
            let ingresses: Api<Ingress> = Api::namespaced(self.client.clone(), &namespace);
            let jobs: Api<Job> = Api::namespaced(self.client.clone(), &namespace);
            let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &namespace);

            for deployment_name in [&api_name, &web_name] {
                if let Err(error) = deployments.delete(deployment_name, &delete_params).await
                    && !is_not_found(&error)
                {
                    return Err(OperatorError::Kube {
                        message: error.to_string(),
                    });
                }
            }
            for service_name in [&api_name, &web_name] {
                if let Err(error) = services.delete(service_name, &delete_params).await
                    && !is_not_found(&error)
                {
                    return Err(OperatorError::Kube {
                        message: error.to_string(),
                    });
                }
            }
            if let Err(error) = ingresses.delete(&name, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }
            remove_edge(self.client.clone(), &name, &namespace).await?;
            // Background rather than the server's default: a Job deleted
            // without a policy orphans its pods, which then sit in a namespace
            // whose instance is gone and which nothing will ever collect.
            if let Err(error) = jobs
                .delete_collection(
                    &kube::api::DeleteParams {
                        propagation_policy: Some(kube::api::PropagationPolicy::Background),
                        ..Default::default()
                    },
                    &kube::api::ListParams::default().labels(&migration_jobs),
                )
                .await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }
            if let Some(cluster) = shared.as_ref() {
                self.drop_shared_database(instance, cluster).await?;
            }
            if let Err(error) = secrets.delete(&db_secret_name, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }

            if shared.is_none()
                && let Err(error) = clusters.delete(&cluster_name, &delete_params).await
                && !is_not_found(&error)
            {
                return Err(OperatorError::Kube {
                    message: error.to_string(),
                });
            }

            warn!(
                name = %name,
                namespace = %namespace,
                provider = "ferriskey",
                "Ferriskey cleanup removed runtime and managed CNPG resources"
            );
            Ok(())
        })
    }

    fn ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a> {
        Box::pin(async move { self.ferriskey_runtime_ready(instance).await })
    }

    fn database_ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a> {
        Box::pin(async move {
            if instance.spec.database.mode == DatabaseMode::Shared {
                return self.shared_database_ready(instance).await;
            }
            self.cnpg_cluster_ready(instance).await
        })
    }

    fn edge_ready<'a>(&'a self, instance: &'a IdentityInstance) -> ProviderReadyFuture<'a> {
        Box::pin(async move { self.ferriskey_route_ready(instance).await })
    }
}

#[derive(Clone)]
struct OperatorContext<S, D, W, A> {
    service: Arc<S>,
    deployer: Arc<D>,
    webhooks: Arc<W>,
    agents: Arc<A>,
    client: Client,
}

const WEBHOOK_RETRY_SECONDS: u64 = 15;
const AGENTS_RETRY_SECONDS: u64 = 15;

/// A webhook that failed or is waiting brings the next look forward; it never
/// turns the instance's own reconcile into a failure.
fn with_webhook_outcome(outcome: ReconcileOutcome, webhooks: WebhookOutcome) -> ReconcileOutcome {
    match webhooks {
        WebhookOutcome::Settled => outcome,
        WebhookOutcome::Retry => {
            let soon = Duration::from_secs(WEBHOOK_RETRY_SECONDS);
            ReconcileOutcome::requeue_after(outcome.requeue_after.map_or(soon, |at| at.min(soon)))
        }
    }
}

fn with_agents_outcome(outcome: ReconcileOutcome, agents: AgentsOutcome) -> ReconcileOutcome {
    match agents {
        AgentsOutcome::Settled => outcome,
        AgentsOutcome::Revisit(later) => {
            ReconcileOutcome::requeue_after(outcome.requeue_after.map_or(later, |at| at.min(later)))
        }
        AgentsOutcome::Retry => {
            let soon = Duration::from_secs(AGENTS_RETRY_SECONDS);
            ReconcileOutcome::requeue_after(outcome.requeue_after.map_or(soon, |at| at.min(soon)))
        }
    }
}

async fn reconcile<S, D, W, A>(
    instance: Arc<IdentityInstance>,
    context: Arc<OperatorContext<S, D, W, A>>,
) -> Result<Action, OperatorError>
where
    S: IdentityInstanceService,
    D: IdentityInstanceDeployer,
    W: IdentityInstanceWebhooks,
    A: IdentityInstanceAgents,
{
    if instance.metadata.deletion_timestamp.is_some() {
        return handle_deletion(instance, context).await;
    }

    ensure_finalizer(&instance, &context.client).await?;
    let name = instance.metadata.name.clone().unwrap_or_default();
    let namespace = instance.metadata.namespace.clone().unwrap_or_default();
    info!(
        name = %name,
        namespace = %namespace,
        "Reconciling IdentityInstance"
    );

    let outcome = context.service.reconcile((*instance).clone()).await?;
    let webhooks = context.webhooks.reconcile(&instance).await;
    let agents = context.agents.reconcile(&instance).await;
    Ok(outcome_to_action(with_agents_outcome(
        with_webhook_outcome(outcome, webhooks),
        agents,
    )))
}

fn error_policy<S, D, W, A>(
    _instance: Arc<IdentityInstance>,
    error: &OperatorError,
    _context: Arc<OperatorContext<S, D, W, A>>,
) -> Action {
    error!(error = %error, "Reconcile error");
    error_requeue_action()
}

fn error_requeue_action() -> Action {
    Action::requeue(Duration::from_secs(30))
}

fn outcome_to_action(outcome: ReconcileOutcome) -> Action {
    match outcome.requeue_after {
        Some(delay) => Action::requeue(delay),
        None => Action::await_change(),
    }
}

pub async fn run() -> Result<(), OperatorError> {
    info!("Starting Autharie operator");
    let client = Client::try_default()
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;
    // Read before anything watches: an operator that cannot name its Gateway
    // would reconcile every instance into a route attached to nothing, and the
    // instances would look healthy while serving no traffic.
    let edge = Edge::from_env()?;
    info!(
        gateway = %edge.gateway_name,
        namespace = %edge.gateway_namespace,
        "tenant routes will attach to this gateway"
    );

    // Absent is the default and a legitimate one: an installation that has
    // not turned tracing on provisions FerrisKey exactly as it always has.
    let otlp_endpoint = std::env::var("HERALD_OTLP_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(endpoint) = otlp_endpoint.as_deref() {
        info!(%endpoint, "every provisioned FerrisKey instance will export traces here");
    }

    // Absent is the default and the common case: a Gateway whose HTTPS
    // listener really is reachable on 443. Only a local cluster remapping
    // it to a host port needs to say so -- see `ferriskey_webapp_url`'s own
    // comment on what goes wrong for a browser when this is wrong.
    let public_https_port = std::env::var("AUTHARIE_PUBLIC_HTTPS_PORT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u16>().ok());
    if let Some(port) = public_https_port {
        info!(
            port,
            "every provisioned FerrisKey instance is reached on this HTTPS port"
        );
    }

    let shared_cluster = SharedCluster::from_env();
    if let Some(cluster) = shared_cluster.as_ref() {
        info!(
            cluster = %cluster.name,
            namespace = %cluster.namespace,
            "shared-mode instances keep their database on this cluster"
        );
    }

    let repository = Arc::new(KubeIdentityInstanceRepository::new(client.clone()));
    let webhooks = Arc::new(WebhookReconciler::new(
        Arc::new(KubeWebhookStatus::new(client.clone())),
        Arc::new(KubeFerriskeyWebhooks::new(client.clone())),
        Arc::new(KubeWebhookSecrets::new(client.clone())),
        Arc::new(KubeWebhookDeliveries::new(client.clone())),
    ));
    let ferriskey_agents = Arc::new(KubeFerriskeyAgents::new(client.clone()));
    let agent_servers = Arc::new(
        KubeAgentServers::new(Arc::clone(&ferriskey_agents)).map_err(|error| {
            OperatorError::Configuration {
                message: format!(
                    "the HTTP client for customer MCP servers cannot be built: {error}"
                ),
            }
        })?,
    );
    let agents = Arc::new(AgentsReconciler::new(
        ferriskey_agents,
        Arc::new(KubeAgentsSecrets::new(client.clone())),
        Arc::new(KubeAgentsStatus::new(client.clone())),
        Arc::new(KubeAgentInstances::new(client.clone())),
        Arc::new(KubeGatewayConfig::new(
            client.clone(),
            edge.gateway_namespace.clone(),
        )),
        Arc::new(KubeAgentsEdge::new(
            client.clone(),
            edge.clone(),
            GatewayService::from_env(),
        )),
        agent_servers,
        AgentsEnvironment::new(edge.gateway_namespace.clone(), public_https_port),
    ));
    let deployer = Arc::new(KubeIdentityInstanceDeployer::new(
        client.clone(),
        edge,
        otlp_endpoint,
        public_https_port,
        shared_cluster,
    ));
    let theme = Arc::new(KubeInstanceTheme::new(client.clone()));
    let service = Arc::new(OperatorApplication::new(
        repository,
        deployer.clone(),
        theme,
    ));

    let instances = Api::<IdentityInstance>::all(client.clone());
    let deployments = Api::<Deployment>::all(client.clone());
    let routes = Api::<DynamicObject>::all_with(client.clone(), &httproute_api_resource());
    let context = Arc::new(OperatorContext {
        service,
        deployer,
        webhooks,
        agents,
        client: client.clone(),
    });

    Controller::new(instances, watcher::Config::default())
        .owns(deployments, watcher::Config::default())
        .owns_with(routes, httproute_api_resource(), watcher::Config::default())
        .run(
            reconcile::<
                OperatorApplication<
                    KubeIdentityInstanceRepository,
                    KubeIdentityInstanceDeployer,
                    KubeInstanceTheme,
                >,
                KubeIdentityInstanceDeployer,
                WebhookReconciler<
                    KubeWebhookStatus,
                    KubeFerriskeyWebhooks,
                    KubeWebhookSecrets,
                    KubeWebhookDeliveries,
                >,
                AgentsReconciler<
                    KubeFerriskeyAgents,
                    KubeAgentsSecrets,
                    KubeAgentsStatus,
                    KubeAgentInstances,
                    KubeGatewayConfig,
                    KubeAgentsEdge,
                    KubeAgentServers,
                >,
            >,
            error_policy::<
                OperatorApplication<
                    KubeIdentityInstanceRepository,
                    KubeIdentityInstanceDeployer,
                    KubeInstanceTheme,
                >,
                KubeIdentityInstanceDeployer,
                WebhookReconciler<
                    KubeWebhookStatus,
                    KubeFerriskeyWebhooks,
                    KubeWebhookSecrets,
                    KubeWebhookDeliveries,
                >,
                AgentsReconciler<
                    KubeFerriskeyAgents,
                    KubeAgentsSecrets,
                    KubeAgentsStatus,
                    KubeAgentInstances,
                    KubeGatewayConfig,
                    KubeAgentsEdge,
                    KubeAgentServers,
                >,
            >,
            context,
        )
        .for_each(|_| async {})
        .await;

    Ok(())
}

const FINALIZER_NAME: &str = "autharie.fr/identityinstance-cleanup";

async fn ensure_finalizer(
    instance: &IdentityInstance,
    client: &Client,
) -> Result<(), OperatorError> {
    let name = instance
        .metadata
        .name
        .clone()
        .ok_or(OperatorError::MissingName)?;
    let namespace = instance
        .metadata
        .namespace
        .clone()
        .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;
    let mut finalizers = instance.metadata.finalizers.clone().unwrap_or_default();
    if finalizers.iter().any(|item| item == FINALIZER_NAME) {
        return Ok(());
    }

    finalizers.push(FINALIZER_NAME.to_string());
    let api: Api<IdentityInstance> = Api::namespaced(client.clone(), &namespace);
    let patch = json!({ "metadata": { "finalizers": finalizers } });
    api.patch(
        &name,
        &kube::api::PatchParams::default(),
        &kube::api::Patch::Merge(&patch),
    )
    .await
    .map_err(|error| OperatorError::Kube {
        message: error.to_string(),
    })?;

    Ok(())
}

async fn handle_deletion<S, D, W, A>(
    instance: Arc<IdentityInstance>,
    context: Arc<OperatorContext<S, D, W, A>>,
) -> Result<Action, OperatorError>
where
    S: IdentityInstanceService,
    D: IdentityInstanceDeployer,
    A: IdentityInstanceAgents,
{
    let name = instance.metadata.name.clone().unwrap_or_default();
    let namespace = instance.metadata.namespace.clone().unwrap_or_default();

    context.agents.cleanup(&instance).await;

    context
        .deployer
        .cleanup_provider_resources(&instance)
        .await?;

    let mut finalizers = instance.metadata.finalizers.clone().unwrap_or_default();
    finalizers.retain(|item| item != FINALIZER_NAME);
    let api: Api<IdentityInstance> = Api::namespaced(context.client.clone(), &namespace);
    let patch = json!({ "metadata": { "finalizers": finalizers } });
    api.patch(
        &name,
        &kube::api::PatchParams::default(),
        &kube::api::Patch::Merge(&patch),
    )
    .await
    .map_err(|error| OperatorError::Kube {
        message: error.to_string(),
    })?;

    info!(
        name = %name,
        namespace = %namespace,
        "Cleanup completed, finalizer removed"
    );

    Ok(Action::await_change())
}

fn is_not_found(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(api_error) if api_error.code == 404)
}

fn keycloak_labels(instance: &IdentityInstance) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert("app.kubernetes.io/name".to_string(), "keycloak".to_string());
    labels.insert(
        "app.kubernetes.io/instance".to_string(),
        instance.metadata.name.clone().unwrap_or_default(),
    );
    labels
}

fn ferriskey_labels(instance: &IdentityInstance, component: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "ferriskey".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        component.to_string(),
    );
    labels.insert(
        "app.kubernetes.io/instance".to_string(),
        instance.metadata.name.clone().unwrap_or_default(),
    );
    labels
}

/// The port ferriskey-api listens on, and the only port its Service
/// publishes. Named once so the container, the Service, and the webapp's
/// fallback base URL cannot name three different ports between them, which is
/// exactly how the fallback used to end up pointing at 8080 with nothing
/// behind it.
pub(crate) const FERRISKEY_API_PORT: i32 = 3333;
pub(crate) const FERRISKEY_API_ROOT_PATH: &str = "/api";

/// Shared with the upgrade controller, which probes this Service to tell a
/// pod that is ready from a product that is serving. A second copy of the
/// convention would drift, and a probe pointed at a name nothing answers on
/// now costs a rollback rather than a warning.
pub(crate) fn ferriskey_api_name(instance_name: &str) -> String {
    format!("{instance_name}-api")
}

fn ferriskey_webapp_name(instance_name: &str) -> String {
    format!("{instance_name}-webapp")
}

fn ferriskey_migration_selector(instance: &IdentityInstance) -> String {
    ferriskey_labels(instance, "migrations")
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One migration job per version, not one per instance.
///
/// The name used to be `{instance}-migrate`, and the reconciler skips a job it
/// already finds, so migrations ran once at install and never again: an
/// upgrade patched the image and the new code met the old schema. The version
/// in the name is what makes the next one a different job.
///
/// Kept under the 63 characters a job name allows by trimming the instance,
/// never the version: two jobs that differ only past the limit would collide,
/// and the one that collides is the one nobody ran.
fn ferriskey_migration_job_name(instance: &IdentityInstance) -> String {
    let instance_name = instance.metadata.name.clone().unwrap_or_default();
    migration_job_name(&instance_name, &instance.spec.version)
}

const MAX_JOB_NAME: usize = 63;

fn migration_job_name(instance_name: &str, version: &str) -> String {
    let version = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>();

    let suffix = format!("-migrate-{version}");
    let room = MAX_JOB_NAME.saturating_sub(suffix.len());
    let head: String = instance_name.chars().take(room).collect();

    format!("{}{suffix}", head.trim_end_matches('-'))
}

fn ferriskey_db_credentials_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-ferriskey-db")
}

pub(crate) fn ferriskey_api_admin_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-api-admin")
}

fn ferriskey_database_host(instance: &IdentityInstance, namespace: &str) -> String {
    format!(
        "{}-rw.{}.svc.cluster.local",
        cnpg_cluster_name(instance),
        namespace
    )
}

fn ferriskey_api_image(instance: &IdentityInstance) -> String {
    format!("ghcr.io/ferriskey/ferriskey-api:{}", instance.spec.version)
}

fn ferriskey_webapp_image(instance: &IdentityInstance) -> String {
    format!(
        "ghcr.io/ferriskey/ferriskey-webapp:{}",
        instance.spec.version
    )
}

/// The origin a browser reaches this Gateway's HTTPS listener on: the
/// instance's own hostname, with an explicit port only where 443 is not
/// really 443 -- see [`KubeIdentityInstanceDeployer::new`]'s comment on
/// `public_https_port` for why that is a real, local-only case rather than
/// a hedge against one that cannot happen.
pub(crate) fn public_origin(hostname: &str, public_https_port: Option<u16>) -> String {
    match public_https_port {
        Some(port) => format!("https://{hostname}:{port}"),
        None => format!("https://{hostname}"),
    }
}

/// `instance.spec.hostname` is what the `HTTPRoute` actually serves this
/// instance on -- required on the CRD, never empty -- so it is what both
/// URLs below default to rather than an address only reachable from inside
/// the cluster (the API's own Service DNS name) or a production domain no
/// local instance was ever given (`{instance_name}.autharie.rs`). Either
/// default is only used when the control plane has not sent an explicit
/// override.
fn ferriskey_webapp_url(instance: &IdentityInstance, public_https_port: Option<u16>) -> String {
    instance
        .spec
        .ferriskey
        .as_ref()
        .and_then(|config| config.webapp_url.as_ref())
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| public_origin(&instance.spec.hostname, public_https_port))
}

const POOLED_CELL_API_BASE_URL: &str = "/api";
const WEBAPP_URL_FROM_REQUEST: &str = "WEBAPP_URL_FROM_REQUEST";

fn is_pooled_cell(instance: &IdentityInstance) -> bool {
    instance
        .spec
        .ferriskey
        .as_ref()
        .is_some_and(|config| config.pooled_cell)
}

const WEBAPP_ALLOWED_HOSTS: &str = "WEBAPP_ALLOWED_HOSTS";

fn webapp_allowed_hosts(hostname: &str) -> String {
    let hostname = hostname.trim();

    match hostname.split_once('.') {
        Some((_, zone)) if !zone.is_empty() => format!("*.{zone}"),
        _ => hostname.to_string(),
    }
}

const DATABASE_NAME_ENV: &str = "DATABASE_NAME";

fn is_recreated(deployment: &Deployment) -> bool {
    deployment
        .spec
        .as_ref()
        .and_then(|spec| spec.strategy.as_ref())
        .and_then(|strategy| strategy.type_.as_deref())
        == Some("Recreate")
}

/// The API server fills in `rollingUpdate` on every Deployment, and a
/// server-side apply that switches to `Recreate` leaves it there, which the API
/// server then refuses. It is removed with a merge patch first. A Deployment
/// not created yet has nothing to clear.
async fn clear_rolling_update(
    deployments: &Api<Deployment>,
    name: &str,
) -> Result<(), OperatorError> {
    let patch = serde_json::json!({
        "spec": { "strategy": { "type": "Recreate", "rollingUpdate": null } }
    });
    match deployments
        .patch(
            name,
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&patch),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(response)) if response.code == 404 => Ok(()),
        Err(error) => Err(OperatorError::Kube {
            message: error.to_string(),
        }),
    }
}

/// On the shared cluster a tenant's role may hold `ROLE_CONNECTION_LIMIT`
/// connections, and FerrisKey keeps 5 open from start. A rolling update runs
/// the new pod beside the old one, which already holds most of them, so the new
/// one times out on its pool and the rollout never completes. A Free instance
/// has one replica either way, so it is replaced instead.
fn replace_rather_than_roll(api: &mut Deployment) {
    if let Some(spec) = api.spec.as_mut() {
        spec.strategy = Some(DeploymentStrategy {
            type_: Some("Recreate".to_string()),
            rolling_update: None,
        });
    }
}

fn use_database_name(api: &mut Deployment, database: &str) {
    let containers = api
        .spec
        .iter_mut()
        .filter_map(|spec| spec.template.spec.as_mut())
        .flat_map(|pod| pod.containers.iter_mut());

    for container in containers {
        for var in container
            .env
            .iter_mut()
            .flatten()
            .filter(|var| var.name == DATABASE_NAME_ENV)
        {
            var.value = Some(database.to_string());
        }
    }
}

const AUTHZ_ENABLED_ENV: &str = "AUTHZ_ENABLED";

fn enable_authorization(api: &mut Deployment) {
    let containers = api
        .spec
        .iter_mut()
        .filter_map(|spec| spec.template.spec.as_mut())
        .flat_map(|pod| pod.containers.iter_mut());

    for container in containers {
        container.env.get_or_insert_with(Vec::new).push(EnvVar {
            name: AUTHZ_ENABLED_ENV.to_string(),
            value: Some("true".to_string()),
            ..Default::default()
        });
    }
}

fn derive_webapp_url_from_request(api: &mut Deployment, hostname: &str) {
    let allowed_hosts = webapp_allowed_hosts(hostname);
    let containers = api
        .spec
        .iter_mut()
        .filter_map(|spec| spec.template.spec.as_mut())
        .flat_map(|pod| pod.containers.iter_mut());

    for container in containers {
        container.env.get_or_insert_with(Vec::new).extend([
            EnvVar {
                name: WEBAPP_URL_FROM_REQUEST.to_string(),
                value: Some("true".to_string()),
                ..Default::default()
            },
            EnvVar {
                name: WEBAPP_ALLOWED_HOSTS.to_string(),
                value: Some(allowed_hosts.clone()),
                ..Default::default()
            },
        ]);
    }
}

pub(crate) fn ferriskey_api_base_url(
    instance: &IdentityInstance,
    public_https_port: Option<u16>,
) -> String {
    if is_pooled_cell(instance) {
        return POOLED_CELL_API_BASE_URL.to_string();
    }

    instance
        .spec
        .ferriskey
        .as_ref()
        .and_then(|config| config.api_base_url.as_ref())
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}/api",
                public_origin(&instance.spec.hostname, public_https_port)
            )
        })
}

fn ferriskey_allowed_origins(webapp_url: &str) -> String {
    const LOCAL_WEBAPP_ORIGIN: &str = "http://localhost:5555";
    if webapp_url == LOCAL_WEBAPP_ORIGIN {
        LOCAL_WEBAPP_ORIGIN.to_string()
    } else {
        format!("{webapp_url},{LOCAL_WEBAPP_ORIGIN}")
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_route(
    client: Client,
    instance: &IdentityInstance,
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    owner_reference: Option<OwnerReference>,
    edge: &Edge,
    backends: &[Backend],
) -> Result<(), OperatorError> {
    // Cloned rather than moved: both the route and the policy hang off the
    // instance, so the second one is deleted with it too.
    let owner = owner_reference.clone();
    let route = build_route(
        instance,
        name,
        namespace,
        labels,
        owner_reference,
        edge,
        backends,
    );
    let routes: Api<DynamicObject> =
        Api::namespaced_with(client.clone(), namespace, &httproute_api_resource());

    routes
        .patch(
            name,
            &kube::api::PatchParams::apply("autharie-operator").force(),
            &kube::api::Patch::Apply(&route),
        )
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;

    apply_network_policy(client.clone(), instance, name, namespace, labels, owner).await?;

    // Only once the route is serving. An instance created before the move to
    // Gateway API still has an Ingress pointing at the same hostname, and
    // removing it first would take the instance off the air for as long as the
    // route takes to be programmed.
    remove_superseded_ingress(client, name, namespace).await
}

/// Applies the allow list, or removes the policy when there is none.
///
/// An open instance has no policy at all rather than one permitting
/// 0.0.0.0/0. A policy that exists and allows everything is indistinguishable
/// from a rule somebody got wrong, and reading `kubectl get securitypolicy`
/// should answer "is this instance restricted" without opening anything.
async fn apply_network_policy(
    client: Client,
    instance: &IdentityInstance,
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    owner_reference: Option<OwnerReference>,
) -> Result<(), OperatorError> {
    let policies: Api<DynamicObject> =
        Api::namespaced_with(client, namespace, &security_policy_api_resource());
    let ranges = allowed_ranges(instance);

    if ranges.is_empty() {
        return match policies
            .delete(name, &kube::api::DeleteParams::default())
            .await
        {
            Ok(_) => {
                info!(name = %name, namespace = %namespace, "the instance is open again; policy removed");
                Ok(())
            }
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(OperatorError::Kube {
                message: error.to_string(),
            }),
        };
    }

    let policy = build_security_policy(name, namespace, labels, owner_reference, ranges);
    policies
        .patch(
            name,
            &kube::api::PatchParams::apply("autharie-operator").force(),
            &kube::api::Patch::Apply(&policy),
        )
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;

    Ok(())
}

/// Removes the route and the policy hung off it.
///
/// Both carry an owner reference, so garbage collection would get them
/// eventually. Eventually is not the same as before the finalizer comes off:
/// a route outliving its instance keeps a hostname claimed, and a policy
/// outliving its route is a rule pointing at nothing.
async fn remove_edge(client: Client, name: &str, namespace: &str) -> Result<(), OperatorError> {
    let params = kube::api::DeleteParams::default();

    for resource in [httproute_api_resource(), security_policy_api_resource()] {
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &resource);

        if let Err(error) = api.delete(name, &params).await
            && !is_not_found(&error)
        {
            return Err(OperatorError::Kube {
                message: error.to_string(),
            });
        }
    }

    Ok(())
}

/// Deletes the Ingress an instance was served through before Gateway API.
///
/// Nothing creates one any more, so on an instance created since this is a
/// no-op. It exists for the ones that predate it, which would otherwise keep
/// two objects claiming the same hostname on two different edges.
async fn remove_superseded_ingress(
    client: Client,
    name: &str,
    namespace: &str,
) -> Result<(), OperatorError> {
    let ingresses: Api<Ingress> = Api::namespaced(client, namespace);

    match ingresses
        .delete(name, &kube::api::DeleteParams::default())
        .await
    {
        Ok(_) => {
            info!(name = %name, namespace = %namespace, "removed the ingress the route replaces");
            Ok(())
        }
        Err(error) if is_not_found(&error) => Ok(()),
        Err(error) => Err(OperatorError::Kube {
            message: error.to_string(),
        }),
    }
}

async fn route_ready_or_unexposed(
    client: Client,
    instance: &IdentityInstance,
) -> Result<bool, OperatorError> {
    if !exposed(instance) {
        return Ok(true);
    }
    let name = instance
        .metadata
        .name
        .clone()
        .ok_or(OperatorError::MissingName)?;
    let namespace = instance
        .metadata
        .namespace
        .clone()
        .ok_or_else(|| OperatorError::MissingNamespace { name: name.clone() })?;

    let routes: Api<DynamicObject> =
        Api::namespaced_with(client, &namespace, &httproute_api_resource());
    let route = routes
        .get_opt(&name)
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;

    let Some(route) = route else {
        return Ok(false);
    };

    Ok(route_is_ready(route.data.get("status")))
}

fn cnpg_base_spec(managed_cluster: &ManagedClusterConfig) -> serde_json::Map<String, Value> {
    let mut spec = serde_json::Map::new();
    spec.insert("instances".to_string(), json!(managed_cluster.instances));
    let mut storage = serde_json::Map::new();
    storage.insert("size".to_string(), json!(managed_cluster.storage.size));
    if let Some(storage_class) = managed_cluster.storage.storage_class.as_ref() {
        storage.insert("storageClass".to_string(), json!(storage_class));
    }
    spec.insert("storage".to_string(), Value::Object(storage));

    if let Some(resources) = cnpg_resources_json(&managed_cluster.resources) {
        spec.insert("resources".to_string(), resources);
    }

    spec
}

const CNPG_HIBERNATION_ANNOTATION: &str = "cnpg.io/hibernation";

fn cnpg_cluster_spec(
    instance: &IdentityInstance,
) -> Result<serde_json::Map<String, Value>, OperatorError> {
    let mut spec = cnpg_base_spec(&instance.spec.database.managed_cluster);

    // Absent when the instance archives nowhere, and left off the spec
    // entirely rather than written as an empty object: an empty
    // barmanObjectStore is a destination of "", which CloudNativePG
    // accepts and then fails on at archive time, hours later.
    if let Some(backup) = cluster_backup_section(instance.spec.backup.as_ref()) {
        spec.insert("backup".to_string(), backup);
    }

    // A recovery reads somebody else's prefix to come up.
    //
    // Written on every apply, not only on the one that creates the
    // cluster. This is a server-side apply with force: a field left out is
    // a field *removed*, so skipping it on later reconciles took
    // `bootstrap` off the object and CloudNativePG defaulted it back to
    // `initdb` -- a recovery that came up empty, which is the failure this
    // whole path exists to prevent. Re-asserting it costs nothing:
    // CloudNativePG reads `bootstrap` once, when the cluster is created.
    if let Some(restore) = instance.spec.restore.as_ref() {
        let backup = instance.spec.backup.as_ref().ok_or_else(|| {
            // A recovery needs credentials for the store it reads, and
            // the ones it archives with are the same. An instance
            // restoring into an installation that archives nowhere has
            // nothing to read with.
            OperatorError::Kube {
                message: "a recovery needs an archive configuration to read with".to_string(),
            }
        })?;

        let (bootstrap, external) = cluster_recovery_section(
            restore,
            &backup.credentials_secret,
            backup.endpoint_url.as_deref(),
        );

        spec.insert("bootstrap".to_string(), bootstrap);
        spec.insert("externalClusters".to_string(), external);
    }

    Ok(spec)
}

/// "off" rather than absent, so resuming is a value the operator sets and not
/// the removal of one.
fn cnpg_cluster_manifest(
    instance: &IdentityInstance,
    namespace: &str,
    owner_reference: Option<OwnerReference>,
    hibernate: bool,
) -> Result<Value, OperatorError> {
    let hibernation = if hibernate { "on" } else { "off" };
    Ok(json!({
        "apiVersion": "postgresql.cnpg.io/v1",
        "kind": "Cluster",
        "metadata": {
            "name": cnpg_cluster_name(instance),
            "namespace": namespace,
            "ownerReferences": owner_reference.map(|owner| vec![owner]),
            "annotations": { CNPG_HIBERNATION_ANNOTATION: hibernation },
        },
        "spec": cnpg_cluster_spec(instance)?
    }))
}

async fn apply_managed_db_cluster(
    client: &Client,
    instance: &IdentityInstance,
    namespace: &str,
    owner_reference: Option<OwnerReference>,
    hibernate: bool,
) -> Result<(), OperatorError> {
    // The credentials go in first. A cluster referring to a secret that is
    // not there yet does come up, and then fails every archive until
    // somebody notices, which is a worse failure than not coming up.
    ensure_archive_credentials(client, instance, namespace).await?;

    let manifest = cnpg_cluster_manifest(instance, namespace, owner_reference, hibernate)?;

    if let Some(restore) = instance.spec.restore.as_ref() {
        info!(
            source = %restore.server_name,
            from = %restore.destination_path,
            "bootstrapping from an archive"
        );
    }

    let clusters: Api<DynamicObject> = Api::namespaced_with(
        client.clone(),
        namespace,
        &ApiResource::from_gvk(&GroupVersionKind::gvk(
            "postgresql.cnpg.io",
            "v1",
            "Cluster",
        )),
    );
    clusters
        .patch(
            &cnpg_cluster_name(instance),
            &kube::api::PatchParams::apply("autharie-operator").force(),
            &kube::api::Patch::Apply(&manifest),
        )
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;

    Ok(())
}

/// How long a stopped instance waits for a migration that has not ended before it hibernates
/// anyway: a Job stuck Pending would otherwise keep the database running for ever.
const MIGRATION_WAIT: Duration = Duration::from_secs(30 * 60);

/// Whether the cluster may be put to sleep: the instance is stopped, its Deployments have no
/// pod left, as far as their status reports terminating ones, and no migration is still using
/// the database.
///
/// Scaling to zero is asynchronous and a migration Job is not scaled at all, so
/// hibernating on the stop alone takes the database from under both.
fn hibernation_due(
    instance: &IdentityInstance,
    deployments: &[Deployment],
    jobs: &[Job],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    instance.spec.stopped
        && deployments
            .iter()
            .all(|deployment| replicas_ready(deployment, 0))
        && !jobs.iter().any(|job| migration_holds_hibernation(job, now))
}

fn migration_holds_hibernation(job: &Job, now: chrono::DateTime<chrono::Utc>) -> bool {
    let overdue = job
        .metadata
        .creation_timestamp
        .as_ref()
        .is_some_and(|created| {
            (now - created.0)
                .to_std()
                .is_ok_and(|age| age > MIGRATION_WAIT)
        });
    migration_running(job) && !overdue
}

fn job_has_condition(job: &Job, kind: &str) -> bool {
    job.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == kind && condition.status == "True")
        })
}

fn migration_failed(job: &Job) -> bool {
    job_has_condition(job, "Failed")
}

/// A Job that has not been given a status yet is running: counting only
/// `status.active` would let a migration created a moment ago look finished.
fn migration_running(job: &Job) -> bool {
    let succeeded = job
        .status
        .as_ref()
        .and_then(|status| status.succeeded)
        .unwrap_or(0);
    succeeded == 0 && !job_has_condition(job, "Complete") && !migration_failed(job)
}

/// The status still says Stopped on the pass that follows the start, because the status is
/// patched only after the resources are ensured. A pass that cannot reach the migration step
/// yet, the database being asleep, still gets here first.
fn restarting(instance: &IdentityInstance) -> bool {
    !instance.spec.stopped
        && instance
            .status
            .as_ref()
            .is_some_and(|status| status.phase == Some(Phase::Stopped))
}

async fn existing_deployments(
    client: &Client,
    namespace: &str,
    names: &[String],
) -> Result<Vec<Deployment>, OperatorError> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let mut found = Vec::new();
    for name in names {
        let deployment = api
            .get_opt(name)
            .await
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })?;
        found.extend(deployment);
    }
    Ok(found)
}

async fn deployment_ready(
    api: &Api<Deployment>,
    name: &str,
    desired_replicas: i32,
) -> Result<bool, OperatorError> {
    let deployment = api
        .get_opt(name)
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;
    let Some(deployment) = deployment else {
        return Ok(false);
    };

    Ok(replicas_ready(&deployment, desired_replicas))
}

/// Keycloak is not configured as a cluster, so more than one replica of it
/// would not share sessions: only a FerrisKey instance runs the number asked.
///
/// A stopped instance runs none, whichever provider it is.
pub(crate) fn desired_replicas(instance: &IdentityInstance) -> i32 {
    if instance.spec.stopped {
        return 0;
    }
    match instance.spec.provider {
        IdentityProvider::Keycloak => 1,
        IdentityProvider::Ferriskey => instance.spec.replicas,
    }
}

fn spread_across_nodes(labels: &BTreeMap<String, String>, replicas: i32) -> Option<Affinity> {
    (replicas > 1).then(|| Affinity {
        pod_anti_affinity: Some(PodAntiAffinity {
            preferred_during_scheduling_ignored_during_execution: Some(vec![
                WeightedPodAffinityTerm {
                    weight: 100,
                    pod_affinity_term: PodAffinityTerm {
                        label_selector: Some(LabelSelector {
                            match_labels: Some(labels.clone()),
                            ..Default::default()
                        }),
                        topology_key: "kubernetes.io/hostname".to_string(),
                        ..Default::default()
                    },
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn replicas_ready(deployment: &Deployment, desired_replicas: i32) -> bool {
    let generation = deployment.metadata.generation.unwrap_or_default();
    let observed_generation = deployment
        .status
        .as_ref()
        .and_then(|status| status.observed_generation)
        .unwrap_or_default();
    let ready_replicas = deployment
        .status
        .as_ref()
        .and_then(|status| status.ready_replicas)
        .unwrap_or(0);
    let available_replicas = deployment
        .status
        .as_ref()
        .and_then(|status| status.available_replicas)
        .unwrap_or(0);

    if desired_replicas == 0 {
        let remaining = deployment.status.as_ref().map_or(0, |status| {
            status.replicas.unwrap_or(0) + status.terminating_replicas.unwrap_or(0)
        });
        return observed_generation >= generation && remaining == 0;
    }

    observed_generation >= generation
        && ready_replicas >= desired_replicas
        && available_replicas >= desired_replicas
}

fn build_keycloak_deployment(
    instance: &IdentityInstance,
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    admin_secret_name: &str,
    owner_reference: Option<OwnerReference>,
) -> Result<Deployment, OperatorError> {
    let image = format!("quay.io/keycloak/keycloak:{}", instance.spec.version);
    let credentials_secret = keycloak_db_credentials_secret_name(name);
    let selector = LabelSelector {
        match_labels: Some(labels.clone()),
        ..Default::default()
    };

    Ok(Deployment {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            owner_references: owner_reference.map(|owner| vec![owner]),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(desired_replicas(instance)),
            selector,
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels.clone()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "keycloak".to_string(),
                        image: Some(image),
                        args: Some(vec![
                            "start-dev".to_string(),
                            "--health-enabled=true".to_string(),
                        ]),
                        ports: Some(vec![
                            ContainerPort {
                                container_port: 8080,
                                ..Default::default()
                            },
                            ContainerPort {
                                container_port: 9000,
                                ..Default::default()
                            },
                        ]),
                        env: Some(vec![
                            EnvVar {
                                name: "KC_DB".to_string(),
                                value: Some("postgres".to_string()),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KC_DB_URL".to_string(),
                                value_from: Some(EnvVarSource {
                                    secret_key_ref: Some(SecretKeySelector {
                                        name: credentials_secret.clone(),
                                        key: "jdbc-uri".to_string(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KC_DB_USERNAME".to_string(),
                                value_from: Some(EnvVarSource {
                                    secret_key_ref: Some(SecretKeySelector {
                                        name: credentials_secret.clone(),
                                        key: "user".to_string(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KC_DB_PASSWORD".to_string(),
                                value_from: Some(EnvVarSource {
                                    secret_key_ref: Some(SecretKeySelector {
                                        name: credentials_secret.clone(),
                                        key: "password".to_string(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KC_HOSTNAME".to_string(),
                                value: Some(instance.spec.hostname.clone()),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KEYCLOAK_ADMIN".to_string(),
                                value_from: Some(EnvVarSource {
                                    secret_key_ref: Some(SecretKeySelector {
                                        name: admin_secret_name.to_string(),
                                        key: "username".to_string(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KEYCLOAK_ADMIN_PASSWORD".to_string(),
                                value_from: Some(EnvVarSource {
                                    secret_key_ref: Some(SecretKeySelector {
                                        name: admin_secret_name.to_string(),
                                        key: "password".to_string(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        ]),
                        startup_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/health/started".to_string()),
                                port:
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                                        9000,
                                    ),
                                scheme: Some("HTTP".to_string()),
                                ..Default::default()
                            }),
                            failure_threshold: Some(60),
                            period_seconds: Some(5),
                            timeout_seconds: Some(2),
                            ..Default::default()
                        }),
                        readiness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/health/ready".to_string()),
                                port:
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                                        9000,
                                    ),
                                scheme: Some("HTTP".to_string()),
                                ..Default::default()
                            }),
                            period_seconds: Some(10),
                            timeout_seconds: Some(2),
                            failure_threshold: Some(6),
                            success_threshold: Some(1),
                            ..Default::default()
                        }),
                        liveness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/health/live".to_string()),
                                port:
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                                        9000,
                                    ),
                                scheme: Some("HTTP".to_string()),
                                ..Default::default()
                            }),
                            period_seconds: Some(10),
                            timeout_seconds: Some(2),
                            failure_threshold: Some(6),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn build_keycloak_service(
    _instance: &IdentityInstance,
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    owner_reference: Option<OwnerReference>,
) -> Result<Service, OperatorError> {
    Ok(Service {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            owner_references: owner_reference.map(|owner| vec![owner]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(labels.clone()),
            ports: Some(vec![ServicePort {
                port: 80,
                target_port: Some(
                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8080),
                ),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// The env vars every FerrisKey API instance gets, whatever this
/// installation's observability settings are -- pulled out of
/// `build_ferriskey_api_deployment` so that function can push the optional
/// OTLP-related vars onto the end without one enormous literal doing both
/// jobs.
fn ferriskey_api_env_vars(
    db_secret_name: &str,
    admin_secret_name: &str,
    db_host: &str,
    webapp_url: &str,
    allowed_origins: &str,
) -> Vec<EnvVar> {
    vec![
        EnvVar {
            name: "DATABASE_HOST".to_string(),
            value: Some(db_host.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "DATABASE_NAME".to_string(),
            value: Some("app".to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "DATABASE_PORT".to_string(),
            value: Some("5432".to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "DATABASE_USER".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: db_secret_name.to_string(),
                    key: "user".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        EnvVar {
            name: "DATABASE_PASSWORD".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: db_secret_name.to_string(),
                    key: "password".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        EnvVar {
            name: "ADMIN_USERNAME".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: admin_secret_name.to_string(),
                    key: "username".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        EnvVar {
            name: "ADMIN_PASSWORD".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: admin_secret_name.to_string(),
                    key: "password".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        EnvVar {
            name: "ADMIN_EMAIL".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: admin_secret_name.to_string(),
                    key: "email".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        EnvVar {
            name: "SERVER_PORT".to_string(),
            value: Some(FERRISKEY_API_PORT.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "SERVER_ROOT_PATH".to_string(),
            value: Some(FERRISKEY_API_ROOT_PATH.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "WEBAPP_URL".to_string(),
            value: Some(webapp_url.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "ALLOWED_ORIGINS".to_string(),
            value: Some(allowed_origins.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "ENV".to_string(),
            value: Some("production".to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "LOG_FILTER".to_string(),
            value: Some("info".to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "LOG_JSON".to_string(),
            value: Some("false".to_string()),
            ..Default::default()
        },
    ]
}

struct FerriskeyRuntime {
    api: Deployment,
    api_service: Service,
    webapp: Deployment,
    webapp_service: Service,
}

fn build_ferriskey_runtime(
    instance: &IdentityInstance,
    namespace: &str,
    owner_reference: Option<OwnerReference>,
    otlp_endpoint: Option<&str>,
    public_https_port: Option<u16>,
    shared: Option<&SharedCluster>,
) -> Result<FerriskeyRuntime, OperatorError> {
    let name = instance
        .metadata
        .name
        .clone()
        .ok_or(OperatorError::MissingName)?;
    let db_secret_name = ferriskey_db_credentials_secret_name(&name);
    let admin_secret_name = ferriskey_api_admin_secret_name(&name);
    let api_name = ferriskey_api_name(&name);
    let web_name = ferriskey_webapp_name(&name);
    let api_labels = ferriskey_labels(instance, "api");
    let web_labels = ferriskey_labels(instance, "webapp");
    let api_image = ferriskey_api_image(instance);
    let web_image = ferriskey_webapp_image(instance);
    let db_host = match shared {
        Some(cluster) => cluster.host(),
        None => ferriskey_database_host(instance, namespace),
    };
    let webapp_url = ferriskey_webapp_url(instance, public_https_port);
    let api_base_url = ferriskey_api_base_url(instance, public_https_port);
    let allowed_origins = ferriskey_allowed_origins(&webapp_url);

    let mut api = build_ferriskey_api_deployment(
        &api_name,
        namespace,
        &api_labels,
        &api_image,
        &db_secret_name,
        &admin_secret_name,
        &db_host,
        &webapp_url,
        &allowed_origins,
        otlp_endpoint,
        desired_replicas(instance),
        owner_reference.clone(),
    )?;
    if is_pooled_cell(instance) {
        derive_webapp_url_from_request(&mut api, &instance.spec.hostname);
    }
    if authz_required(instance) {
        enable_authorization(&mut api);
    }
    if shared.is_some() {
        let database = SharedDatabaseName::for_instance_name(&name)?;
        use_database_name(&mut api, database.as_str());
        replace_rather_than_roll(&mut api);
    }
    let api_service = build_ferriskey_service(
        &api_name,
        namespace,
        &api_labels,
        FERRISKEY_API_PORT,
        FERRISKEY_API_PORT,
        owner_reference.clone(),
    )?;
    let webapp = build_ferriskey_webapp_deployment(
        &web_name,
        namespace,
        &web_labels,
        &web_image,
        &api_base_url,
        desired_replicas(instance),
        owner_reference.clone(),
    )?;
    let webapp_service =
        build_ferriskey_service(&web_name, namespace, &web_labels, 80, 80, owner_reference)?;

    Ok(FerriskeyRuntime {
        api,
        api_service,
        webapp,
        webapp_service,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_ferriskey_api_deployment(
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    image: &str,
    db_secret_name: &str,
    admin_secret_name: &str,
    db_host: &str,
    webapp_url: &str,
    allowed_origins: &str,
    otlp_endpoint: Option<&str>,
    replicas: i32,
    owner_reference: Option<OwnerReference>,
) -> Result<Deployment, OperatorError> {
    let mut env = ferriskey_api_env_vars(
        db_secret_name,
        admin_secret_name,
        db_host,
        webapp_url,
        allowed_origins,
    );

    // Off unless this installation's own Herald was told to accept OTLP
    // traces (`KubeIdentityInstanceDeployer::new`'s own comment). All three
    // vars together: FerrisKey's own `ObservabilityArgs` gates the exporter
    // on `ACTIVE_OBSERVABILITY`, but its startup check requires a
    // `METRICS_ENDPOINT` too the moment observability is active at all --
    // confirmed the hard way, `active_observability=true` with no metrics
    // endpoint refuses to start with "Metrics endpoint is required when
    // observability is active" rather than just skipping metrics export.
    // Herald has no metrics receiver of its own yet, so this points at the
    // same trace endpoint: a metrics export attempt there 404s and is
    // logged, the same as any other collector FerrisKey cannot reach, and
    // does not stop the process the way the missing var does.
    if let Some(endpoint) = otlp_endpoint {
        env.push(EnvVar {
            name: "ACTIVE_OBSERVABILITY".to_string(),
            value: Some("true".to_string()),
            ..Default::default()
        });
        env.push(EnvVar {
            name: "OTLP_ENDPOINT".to_string(),
            value: Some(endpoint.to_string()),
            ..Default::default()
        });
        env.push(EnvVar {
            name: "METRICS_ENDPOINT".to_string(),
            value: Some(endpoint.to_string()),
            ..Default::default()
        });
    }

    Ok(Deployment {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            owner_references: owner_reference.map(|owner| vec![owner]),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels.clone()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    affinity: spread_across_nodes(labels, replicas),
                    containers: vec![Container {
                        name: "ferriskey-api".to_string(),
                        image: Some(image.to_string()),
                        ports: Some(vec![ContainerPort {
                            name: Some("http".to_string()),
                            container_port: FERRISKEY_API_PORT,
                            ..Default::default()
                        }]),
                        env: Some(env),
                        readiness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/api/health/ready".to_string()),
                                port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(
                                    "http".to_string(),
                                ),
                                scheme: Some("HTTP".to_string()),
                                ..Default::default()
                            }),
                            initial_delay_seconds: Some(5),
                            period_seconds: Some(5),
                            timeout_seconds: Some(3),
                            failure_threshold: Some(3),
                            success_threshold: Some(1),
                            ..Default::default()
                        }),
                        liveness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/api/health/live".to_string()),
                                port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(
                                    "http".to_string(),
                                ),
                                scheme: Some("HTTP".to_string()),
                                ..Default::default()
                            }),
                            initial_delay_seconds: Some(30),
                            period_seconds: Some(10),
                            timeout_seconds: Some(5),
                            failure_threshold: Some(3),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn build_ferriskey_webapp_deployment(
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    image: &str,
    api_base_url: &str,
    replicas: i32,
    owner_reference: Option<OwnerReference>,
) -> Result<Deployment, OperatorError> {
    Ok(Deployment {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            owner_references: owner_reference.map(|owner| vec![owner]),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels.clone()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    affinity: spread_across_nodes(labels, replicas),
                    containers: vec![Container {
                        name: "ferriskey-webapp".to_string(),
                        image: Some(image.to_string()),
                        ports: Some(vec![ContainerPort {
                            container_port: 80,
                            ..Default::default()
                        }]),
                        env: Some(vec![EnvVar {
                            name: "API_URL".to_string(),
                            value: Some(api_base_url.to_string()),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn build_ferriskey_service(
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    port: i32,
    target_port: i32,
    owner_reference: Option<OwnerReference>,
) -> Result<Service, OperatorError> {
    Ok(Service {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            owner_references: owner_reference.map(|owner| vec![owner]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(labels.clone()),
            ports: Some(vec![ServicePort {
                port,
                target_port: Some(
                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(target_port),
                ),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn build_ferriskey_migration_job(
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
    image: &str,
    db_secret_name: &str,
    owner_reference: Option<OwnerReference>,
) -> Result<Job, OperatorError> {
    Ok(Job {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            owner_references: owner_reference.map(|owner| vec![owner]),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(3),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels.clone()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("OnFailure".to_string()),
                    containers: vec![Container {
                        name: "migrations".to_string(),
                        image: Some(image.to_string()),
                        command: Some(vec!["sqlx".to_string()]),
                        args: Some(vec![
                            "migrate".to_string(),
                            "run".to_string(),
                            "--source".to_string(),
                            "/usr/local/src/ferriskey/migrations".to_string(),
                        ]),
                        env: Some(vec![EnvVar {
                            name: "DATABASE_URL".to_string(),
                            value_from: Some(EnvVarSource {
                                secret_key_ref: Some(SecretKeySelector {
                                    name: db_secret_name.to_string(),
                                    key: "database-url".to_string(),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn keycloak_admin_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-admin")
}

fn cnpg_cluster_name(instance: &IdentityInstance) -> String {
    let instance_name = instance.metadata.name.clone().unwrap_or_default();
    format!("{instance_name}-db")
}

fn keycloak_db_credentials_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-db-credentials")
}

fn cnpg_resources_json(
    resources: &autharie_crds::common::types::ResourceRequirements,
) -> Option<serde_json::Value> {
    let mut root = serde_json::Map::new();

    if let Some(requests) = resources.requests.as_ref() {
        let mut req = serde_json::Map::new();
        if let Some(cpu) = requests.cpu.as_ref() {
            req.insert("cpu".to_string(), json!(cpu));
        }
        if let Some(memory) = requests.memory.as_ref() {
            req.insert("memory".to_string(), json!(memory));
        }
        if !req.is_empty() {
            root.insert("requests".to_string(), serde_json::Value::Object(req));
        }
    }

    if let Some(limits) = resources.limits.as_ref() {
        let mut lim = serde_json::Map::new();
        if let Some(cpu) = limits.cpu.as_ref() {
            lim.insert("cpu".to_string(), json!(cpu));
        }
        if let Some(memory) = limits.memory.as_ref() {
            lim.insert("memory".to_string(), json!(memory));
        }
        if !lim.is_empty() {
            root.insert("limits".to_string(), serde_json::Value::Object(lim));
        }
    }

    if root.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(root))
    }
}

fn secret_data_value(
    data: &BTreeMap<String, k8s_openapi::ByteString>,
    key: &str,
) -> Option<String> {
    data.get(key)
        .map(|value| String::from_utf8_lossy(&value.0).to_string())
}

fn generate_password(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

/// Writes the secret CloudNativePG reads the archive credentials from, into
/// the instance's own namespace.
///
/// Per namespace rather than one shared secret, because CloudNativePG resolves
/// secret references inside the cluster's namespace and there is no
/// arrangement where a single secret in `autharie-system` serves every tenant.
///
/// Missing configuration is not an error here. A data plane that archives
/// nothing is a decision; the operator says so once at startup and does not
/// repeat it per instance. What it must not do is write a secret with empty
/// credentials, which would produce a cluster that looks configured and fails
/// every archive.
async fn ensure_archive_credentials(
    client: &Client,
    instance: &IdentityInstance,
    namespace: &str,
) -> Result<(), OperatorError> {
    let Some(config) = instance.spec.backup.as_ref() else {
        return Ok(());
    };

    let Some(store) = ArchiveStore::from_env() else {
        return Err(OperatorError::Configuration {
            message: format!(
                "instance `{}` archives to {} and this data plane has no object store credentials",
                instance.metadata.name.as_deref().unwrap_or("?"),
                config.destination_path
            ),
        });
    };

    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let desired = store.credentials_secret(&config.credentials_secret, namespace);

    secrets
        .patch(
            &config.credentials_secret,
            &kube::api::PatchParams::apply("autharie-operator").force(),
            &kube::api::Patch::Apply(&desired),
        )
        .await
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })?;

    Ok(())
}

#[cfg(test)]
mod migration_job_naming {
    use super::{MAX_JOB_NAME, migration_job_name};

    const INSTANCE: &str = "deployment-0629614f-e116-418a-a5aa-eb45c7768cc6";

    /// The bug this exists for: one name for every version meant the
    /// reconciler found the install job and skipped every migration after it,
    /// so an upgrade ran new code against the old schema.
    #[test]
    fn two_versions_are_two_jobs() {
        assert_ne!(
            migration_job_name(INSTANCE, "0.5.0"),
            migration_job_name(INSTANCE, "0.6.0")
        );
    }

    #[test]
    fn the_version_is_readable_in_the_name() {
        assert!(migration_job_name(INSTANCE, "0.6.0").ends_with("-migrate-0-6-0"));
    }

    /// Kubernetes refuses a job name past 63 characters, and a name that is
    /// cut anywhere is a name that can collide with another version's.
    #[test]
    fn a_long_instance_is_trimmed_and_the_version_kept_whole() {
        let long = "a".repeat(120);
        let name = migration_job_name(&long, "10.11.12");

        assert!(name.len() <= MAX_JOB_NAME, "{} characters", name.len());
        assert!(name.ends_with("-migrate-10-11-12"), "{name}");
    }

    /// Trimming must not leave a name ending on the separator, and two
    /// versions must still differ once both have been trimmed.
    #[test]
    fn trimming_keeps_two_versions_apart() {
        let long = "a".repeat(120);

        let one = migration_job_name(&long, "10.11.12");
        let other = migration_job_name(&long, "10.11.13");

        assert_ne!(one, other);
        assert!(!one.contains("--"), "{one}");
    }

    /// Anything a version could carry that a name cannot.
    #[test]
    fn a_version_that_is_not_only_digits_still_makes_a_legal_name() {
        let name = migration_job_name(INSTANCE, "1.2.3-rc.1+build");

        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{name}"
        );
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_emptied_iam_field_is_named_as_null_so_the_old_value_is_cleared() {
        use autharie_crds::v1alpha::identity_instance::{
            IamPhase, IamStatus, IdentityInstanceStatus,
        };

        let status = IdentityInstanceStatus {
            iam: Some(IamStatus {
                phase: IamPhase::Applied,
                message: None,
                applied: None,
                observed_at: None,
            }),
            ..Default::default()
        };

        let patch = status_merge_patch(&status);

        assert_eq!(patch["status"]["iam"]["phase"], "Applied");
        for field in ["message", "applied", "observedAt"] {
            assert_eq!(patch["status"]["iam"][field], Value::Null, "{field}");
            assert!(
                patch["status"]["iam"]
                    .as_object()
                    .unwrap()
                    .contains_key(field)
            );
        }
    }

    #[test]
    fn a_filled_iam_field_keeps_its_value_in_the_patch() {
        use autharie_crds::v1alpha::identity_instance::{
            IamPhase, IamStatus, IdentityInstanceStatus,
        };

        let status = IdentityInstanceStatus {
            iam: Some(IamStatus {
                phase: IamPhase::Pending,
                message: Some("waiting".to_string()),
                applied: Some("{}".to_string()),
                observed_at: Some("2026-10-03T00:00:00Z".to_string()),
            }),
            ..Default::default()
        };

        let patch = status_merge_patch(&status);

        assert_eq!(patch["status"]["iam"]["message"], "waiting");
        assert_eq!(patch["status"]["iam"]["applied"], "{}");
    }

    /// The bug this exists for: an instance that recovered kept the error
    /// and the Ready=False condition of its last failed reconcile beside
    /// `ready: true`, because a merge patch leaves absent fields alone.
    #[test]
    fn a_recovered_status_clears_the_error_and_the_conditions() {
        let patch = status_merge_patch(&Default::default());

        assert_eq!(patch["status"]["error"], Value::Null);
        assert!(patch["status"].as_object().unwrap().contains_key("error"));
        assert_eq!(patch["status"]["conditions"], json!([]));
    }

    #[test]
    fn a_settled_webhook_pass_leaves_the_requeue_alone() {
        let steady = ReconcileOutcome::requeue_after(Duration::from_secs(60));

        let outcome = with_webhook_outcome(steady, WebhookOutcome::Settled);

        assert_eq!(outcome.requeue_after, Some(Duration::from_secs(60)));
    }

    #[test]
    fn a_webhook_retry_brings_the_next_pass_forward_but_never_pushes_it_back() {
        let steady = ReconcileOutcome::requeue_after(Duration::from_secs(60));
        let soon = ReconcileOutcome::requeue_after(Duration::from_secs(5));

        assert_eq!(
            with_webhook_outcome(steady, WebhookOutcome::Retry).requeue_after,
            Some(Duration::from_secs(WEBHOOK_RETRY_SECONDS))
        );
        assert_eq!(
            with_webhook_outcome(soon, WebhookOutcome::Retry).requeue_after,
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            with_webhook_outcome(ReconcileOutcome::default(), WebhookOutcome::Retry).requeue_after,
            Some(Duration::from_secs(WEBHOOK_RETRY_SECONDS))
        );
    }

    #[test]
    fn a_settled_agents_pass_leaves_the_requeue_alone() {
        let steady = ReconcileOutcome::requeue_after(Duration::from_secs(60));

        let outcome = with_agents_outcome(steady, AgentsOutcome::Settled);

        assert_eq!(outcome.requeue_after, Some(Duration::from_secs(60)));
    }

    #[test]
    fn an_agents_retry_brings_the_next_pass_forward_but_never_pushes_it_back() {
        let steady = ReconcileOutcome::requeue_after(Duration::from_secs(60));
        let soon = ReconcileOutcome::requeue_after(Duration::from_secs(5));

        assert_eq!(
            with_agents_outcome(steady, AgentsOutcome::Retry).requeue_after,
            Some(Duration::from_secs(AGENTS_RETRY_SECONDS))
        );
        assert_eq!(
            with_agents_outcome(soon, AgentsOutcome::Retry).requeue_after,
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            with_agents_outcome(ReconcileOutcome::default(), AgentsOutcome::Retry).requeue_after,
            Some(Duration::from_secs(AGENTS_RETRY_SECONDS))
        );
    }

    fn agents_instance(enabled: bool) -> IdentityInstance {
        let mut instance = stopped(IdentityProvider::Ferriskey);
        instance.spec.stopped = false;
        instance.spec.agents = Some(autharie_crds::v1alpha::identity_instance::AgentsSpec {
            enabled,
            realm: "master".to_string(),
            policies: Vec::new(),
            policies_revision: 0,
            servers: Vec::new(),
        });
        instance
    }

    #[test]
    fn a_ferriskey_instance_with_agents_runs_with_authorization_enabled() {
        let runtime =
            build_ferriskey_runtime(&agents_instance(true), "default", None, None, None, None)
                .unwrap();

        assert!(
            env_of(&runtime.api)
                .contains(&(AUTHZ_ENABLED_ENV.to_string(), Some("true".to_string())))
        );
    }

    #[test]
    fn a_ferriskey_instance_without_agents_does_not_get_authorization() {
        let mut without = agents_instance(false);
        without.spec.agents = None;

        for instance in [agents_instance(false), without] {
            let runtime =
                build_ferriskey_runtime(&instance, "default", None, None, None, None).unwrap();

            assert!(
                !env_of(&runtime.api)
                    .iter()
                    .any(|(name, _)| name == AUTHZ_ENABLED_ENV)
            );
        }
    }

    #[test]
    fn authorization_stays_on_while_agents_are_being_cleaned_up() {
        let mut instance = agents_instance(false);
        instance.status = Some(
            autharie_crds::v1alpha::identity_instance::IdentityInstanceStatus {
                agents: Some(autharie_crds::v1alpha::identity_instance::AgentsStatus {
                    phase: autharie_crds::v1alpha::identity_instance::AgentsPhase::Ready,
                    gateway_client_ready: true,
                    observed_revision: 0,
                    mcp_url: None,
                    message: None,
                    policies: Vec::new(),
                    servers: Vec::new(),
                }),
                ..Default::default()
            },
        );

        let runtime =
            build_ferriskey_runtime(&instance, "default", None, None, None, None).unwrap();

        assert!(
            env_of(&runtime.api)
                .iter()
                .any(|(name, _)| name == AUTHZ_ENABLED_ENV)
        );
    }

    #[test]
    fn a_status_without_iam_adds_no_iam_key() {
        let patch = status_merge_patch(&Default::default());

        assert!(patch["status"].get("iam").is_none());
    }
    use super::*;
    use autharie_crds::common::types::ResourceRequirements;
    use autharie_crds::v1alpha::identity_instance::{
        DatabaseConfig, DatabaseMode, FerriskeyConfig, IdentityInstance, IdentityInstanceSpec,
        IdentityProvider, ManagedClusterConfig, ManagedClusterStorage,
    };
    use kube::core::ObjectMeta;
    use kube::error::ErrorResponse;

    fn instance() -> IdentityInstance {
        IdentityInstance {
            metadata: ObjectMeta {
                name: Some("instance-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: IdentityInstanceSpec {
                webhooks: Vec::new(),
                webhooks_revision: 0,
                webhook_requests: Vec::new(),
                agents: None,
                restore: None,
                organisation_id: "org-1".to_string(),
                provider: IdentityProvider::Keycloak,
                version: "25.0.0".to_string(),
                hostname: "auth.acme.test".to_string(),
                replicas: 1,
                stopped: false,
                database: DatabaseConfig {
                    mode: DatabaseMode::ManagedCluster,
                    shared: None,
                    managed_cluster: ManagedClusterConfig {
                        instances: 1,
                        storage: ManagedClusterStorage {
                            size: "10Gi".to_string(),
                            storage_class: None,
                        },
                        resources: ResourceRequirements {
                            requests: None,
                            limits: None,
                        },
                    },
                },
                ferriskey: None,
                ingress: None,
                allowed_cidrs: None,
                iam: None,
                backup: None,
            },
            status: None,
        }
    }

    fn env_value<'a>(container: &'a Container, name: &str) -> Option<&'a EnvVar> {
        container
            .env
            .as_ref()
            .and_then(|envs| envs.iter().find(|env| env.name == name))
    }

    #[test]
    fn outcome_to_action_maps_requeue() {
        let outcome = ReconcileOutcome::requeue_after(Duration::from_secs(5));
        let action = outcome_to_action(outcome);
        assert_eq!(action, Action::requeue(Duration::from_secs(5)));

        let action = outcome_to_action(ReconcileOutcome::default());
        assert_eq!(action, Action::await_change());
    }

    #[test]
    fn error_policy_requeues_after_30s() {
        let action = error_requeue_action();
        assert_eq!(action, Action::requeue(Duration::from_secs(30)));
    }

    #[test]
    fn is_not_found_detects_404() {
        let not_found = kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: "not found".to_string(),
            reason: "NotFound".to_string(),
            code: 404,
        });
        let other = kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: "boom".to_string(),
            reason: "Internal".to_string(),
            code: 500,
        });

        assert!(is_not_found(&not_found));
        assert!(!is_not_found(&other));
    }

    #[test]
    fn keycloak_admin_secret_name_formats() {
        assert_eq!(keycloak_admin_secret_name("instance-1"), "instance-1-admin");
    }

    #[test]
    fn keycloak_db_credentials_secret_name_formats() {
        assert_eq!(
            keycloak_db_credentials_secret_name("instance-1"),
            "instance-1-db-credentials"
        );
    }

    #[test]
    fn generate_password_returns_alphanumeric() {
        let password = generate_password(32);
        assert_eq!(password.len(), 32);
        assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn keycloak_labels_include_instance_name() {
        let instance = instance();
        let labels = keycloak_labels(&instance);

        assert_eq!(
            labels.get("app.kubernetes.io/name").map(String::as_str),
            Some("keycloak")
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(String::as_str),
            Some("instance-1")
        );
    }

    #[test]
    fn build_keycloak_service_sets_ports_and_labels() {
        let instance = instance();
        let labels = keycloak_labels(&instance);
        let service =
            build_keycloak_service(&instance, "instance-1", "default", &labels, None).unwrap();

        let metadata = service.metadata;
        assert_eq!(metadata.name.as_deref(), Some("instance-1"));
        assert_eq!(metadata.namespace.as_deref(), Some("default"));
        assert_eq!(metadata.labels, Some(labels.clone()));

        let spec = service.spec.expect("service spec");
        assert_eq!(spec.selector, Some(labels));
        let ports = spec.ports.expect("service ports");
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].port, 80);
    }

    #[test]
    fn build_keycloak_deployment_sets_env_and_image() {
        let instance = instance();
        let labels = keycloak_labels(&instance);
        let deployment = build_keycloak_deployment(
            &instance,
            "instance-1",
            "default",
            &labels,
            "instance-1-admin",
            None,
        )
        .unwrap();

        let metadata = deployment.metadata;
        assert_eq!(metadata.name.as_deref(), Some("instance-1"));
        assert_eq!(metadata.namespace.as_deref(), Some("default"));
        assert_eq!(metadata.labels, Some(labels.clone()));

        let spec = deployment.spec.expect("deployment spec");
        let template = spec.template;
        let pod_spec = template.spec.expect("pod spec");
        let container = &pod_spec.containers[0];

        assert_eq!(container.name, "keycloak");
        assert_eq!(
            container.image.as_deref(),
            Some("quay.io/keycloak/keycloak:25.0.0")
        );
        assert_eq!(
            container.args.as_ref(),
            Some(&vec![
                "start-dev".to_string(),
                "--health-enabled=true".to_string(),
            ])
        );
        assert!(
            container
                .ports
                .as_ref()
                .map(|ports| ports.iter().any(|port| port.container_port == 9000))
                .unwrap_or(false)
        );

        let kc_db_url = env_value(container, "KC_DB_URL").and_then(|env| env.value_from.as_ref());
        let kc_db_user =
            env_value(container, "KC_DB_USERNAME").and_then(|env| env.value_from.as_ref());
        let kc_db_pass =
            env_value(container, "KC_DB_PASSWORD").and_then(|env| env.value_from.as_ref());
        let kc_host = env_value(container, "KC_HOSTNAME").and_then(|env| env.value.as_deref());
        let admin_user =
            env_value(container, "KEYCLOAK_ADMIN").and_then(|env| env.value_from.as_ref());
        let admin_pass =
            env_value(container, "KEYCLOAK_ADMIN_PASSWORD").and_then(|env| env.value_from.as_ref());

        assert_eq!(kc_host, Some("auth.acme.test"));
        assert!(
            kc_db_url
                .and_then(|source| source.secret_key_ref.as_ref())
                .map(|secret| secret.name == "instance-1-db-credentials" && secret.key == "jdbc-uri")
                .unwrap_or(false)
        );
        assert!(
            kc_db_user
                .and_then(|source| source.secret_key_ref.as_ref())
                .map(|secret| secret.name == "instance-1-db-credentials" && secret.key == "user")
                .unwrap_or(false)
        );
        assert!(
            kc_db_pass
                .and_then(|source| source.secret_key_ref.as_ref())
                .map(|secret| secret.name == "instance-1-db-credentials" && secret.key == "password")
                .unwrap_or(false)
        );
        assert!(
            admin_user
                .and_then(|source| source.secret_key_ref.as_ref())
                .map(|secret| secret.name == "instance-1-admin" && secret.key == "username")
                .unwrap_or(false)
        );
        assert!(
            admin_pass
                .and_then(|source| source.secret_key_ref.as_ref())
                .map(|secret| secret.name == "instance-1-admin" && secret.key == "password")
                .unwrap_or(false)
        );

        let startup_probe = container
            .startup_probe
            .as_ref()
            .and_then(|probe| probe.http_get.as_ref());
        let readiness_probe = container
            .readiness_probe
            .as_ref()
            .and_then(|probe| probe.http_get.as_ref());
        let liveness_probe = container
            .liveness_probe
            .as_ref()
            .and_then(|probe| probe.http_get.as_ref());

        assert_eq!(
            startup_probe.and_then(|get| get.path.as_deref()),
            Some("/health/started")
        );
        assert_eq!(
            startup_probe.map(|get| get.port.clone()),
            Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(9000))
        );
        assert_eq!(
            readiness_probe.and_then(|get| get.path.as_deref()),
            Some("/health/ready")
        );
        assert_eq!(
            readiness_probe.map(|get| get.port.clone()),
            Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(9000))
        );
        assert_eq!(
            liveness_probe.and_then(|get| get.path.as_deref()),
            Some("/health/live")
        );
        assert_eq!(
            liveness_probe.map(|get| get.port.clone()),
            Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(9000))
        );
    }

    fn api_deployment_with(replicas: i32) -> Deployment {
        build_ferriskey_api_deployment(
            "instance-1-api",
            "default",
            &BTreeMap::new(),
            "ghcr.io/ferriskey/ferriskey-api:1.0.0",
            "instance-1-db-credentials",
            "instance-1-admin",
            "instance-1-db.default.svc",
            "https://auth.acme.test",
            "https://auth.acme.test",
            None,
            replicas,
            None,
        )
        .unwrap()
    }

    fn with_status(mut deployment: Deployment, ready: i32) -> Deployment {
        deployment.metadata.generation = Some(1);
        deployment.status = Some(k8s_openapi::api::apps::v1::DeploymentStatus {
            observed_generation: Some(1),
            ready_replicas: Some(ready),
            available_replicas: Some(ready),
            ..Default::default()
        });
        deployment
    }

    #[test]
    fn the_ferriskey_api_deployment_runs_the_replicas_it_is_given() {
        assert_eq!(api_deployment_with(2).spec.unwrap().replicas, Some(2));
        assert_eq!(api_deployment_with(1).spec.unwrap().replicas, Some(1));
    }

    fn webapp_deployment_with(labels: &BTreeMap<String, String>, replicas: i32) -> Deployment {
        build_ferriskey_webapp_deployment(
            "instance-1-webapp",
            "default",
            labels,
            "ghcr.io/ferriskey/ferriskey-webapp:1.0.0",
            "https://auth.acme.test",
            replicas,
            None,
        )
        .unwrap()
    }

    fn affinity_of(deployment: &Deployment) -> Option<Affinity> {
        deployment
            .spec
            .as_ref()?
            .template
            .spec
            .as_ref()?
            .affinity
            .clone()
    }

    fn assert_spreads_over_nodes(affinity: Option<Affinity>, labels: &BTreeMap<String, String>) {
        let terms = affinity
            .expect("two replicas get an affinity")
            .pod_anti_affinity
            .expect("it is an anti-affinity")
            .preferred_during_scheduling_ignored_during_execution
            .expect("it is a preference, not a requirement");

        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].weight, 100);
        assert_eq!(
            terms[0].pod_affinity_term.topology_key,
            "kubernetes.io/hostname"
        );
        assert_eq!(
            terms[0]
                .pod_affinity_term
                .label_selector
                .as_ref()
                .unwrap()
                .match_labels
                .as_ref(),
            Some(labels)
        );
    }

    #[test]
    fn two_replicas_of_each_ferriskey_workload_prefer_different_nodes() {
        let labels = BTreeMap::from([("app".to_string(), "instance-1".to_string())]);
        let api = build_ferriskey_api_deployment(
            "instance-1-api",
            "default",
            &labels,
            "ghcr.io/ferriskey/ferriskey-api:1.0.0",
            "instance-1-db-credentials",
            "instance-1-admin",
            "instance-1-db.default.svc",
            "https://auth.acme.test",
            "https://auth.acme.test",
            None,
            2,
            None,
        )
        .unwrap();

        assert_spreads_over_nodes(affinity_of(&api), &labels);
        assert_spreads_over_nodes(affinity_of(&webapp_deployment_with(&labels, 2)), &labels);
    }

    #[test]
    fn one_replica_of_each_ferriskey_workload_carries_no_affinity() {
        let labels = BTreeMap::from([("app".to_string(), "instance-1".to_string())]);

        assert!(affinity_of(&api_deployment_with(1)).is_none());
        assert!(affinity_of(&webapp_deployment_with(&labels, 1)).is_none());
    }

    #[test]
    fn a_ferriskey_instance_runs_the_replicas_of_its_spec() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;
        instance.spec.replicas = 3;

        assert_eq!(desired_replicas(&instance), 3);
    }

    #[test]
    fn the_ferriskey_webapp_deployment_runs_the_replicas_it_is_given() {
        let deployment = build_ferriskey_webapp_deployment(
            "instance-1-webapp",
            "default",
            &BTreeMap::new(),
            "ghcr.io/ferriskey/ferriskey-webapp:1.0.0",
            "https://auth.acme.test",
            2,
            None,
        )
        .unwrap();

        assert_eq!(deployment.spec.unwrap().replicas, Some(2));
    }

    #[test]
    fn the_keycloak_deployment_always_runs_one_replica() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Keycloak;
        instance.spec.replicas = 2;

        let deployment = build_keycloak_deployment(
            &instance,
            "instance-1",
            "default",
            &BTreeMap::new(),
            "instance-1-admin",
            None,
        )
        .unwrap();

        assert_eq!(deployment.spec.unwrap().replicas, Some(1));
    }

    fn stopped(provider: IdentityProvider) -> IdentityInstance {
        let mut instance = instance();
        instance.spec.provider = provider;
        instance.spec.replicas = 3;
        instance.spec.stopped = true;
        instance
    }

    #[test]
    fn a_stopped_instance_runs_no_replica_whichever_the_provider() {
        for provider in [IdentityProvider::Keycloak, IdentityProvider::Ferriskey] {
            let mut instance = stopped(provider);
            assert_eq!(desired_replicas(&instance), 0);

            instance.spec.stopped = false;
            assert!(desired_replicas(&instance) >= 1);
        }
    }

    #[test]
    fn a_stopped_keycloak_deployment_runs_no_replica() {
        let instance = stopped(IdentityProvider::Keycloak);

        let deployment = build_keycloak_deployment(
            &instance,
            "instance-1",
            "default",
            &BTreeMap::new(),
            "instance-1-admin",
            None,
        )
        .unwrap();

        assert_eq!(deployment.spec.unwrap().replicas, Some(0));
    }

    #[test]
    fn a_stopped_ferriskey_instance_builds_its_workloads_without_replicas() {
        let runtime = build_ferriskey_runtime(
            &stopped(IdentityProvider::Ferriskey),
            "default",
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(runtime.api.spec.as_ref().unwrap().replicas, Some(0));
        assert_eq!(runtime.webapp.spec.as_ref().unwrap().replicas, Some(0));
        assert!(affinity_of(&runtime.api).is_none());
        assert!(affinity_of(&runtime.webapp).is_none());
    }

    #[test]
    fn a_started_ferriskey_instance_builds_the_replicas_of_its_spec() {
        let mut instance = stopped(IdentityProvider::Ferriskey);
        instance.spec.stopped = false;

        let runtime =
            build_ferriskey_runtime(&instance, "default", None, None, None, None).unwrap();

        assert_eq!(runtime.api.spec.as_ref().unwrap().replicas, Some(3));
        assert_eq!(runtime.webapp.spec.as_ref().unwrap().replicas, Some(3));
    }

    fn hibernation_of(manifest: &Value) -> Option<&str> {
        manifest["metadata"]["annotations"]["cnpg.io/hibernation"].as_str()
    }

    fn recovering(mut instance: IdentityInstance) -> IdentityInstance {
        use autharie_crds::v1alpha::identity_instance::{BackupConfig, RestoreConfig};

        instance.spec.backup = Some(BackupConfig {
            destination_path: "s3://autharie-backups/org-1/instance-1".to_string(),
            endpoint_url: None,
            credentials_secret: "autharie-object-store".to_string(),
            encryption: None,
        });
        instance.spec.restore = Some(RestoreConfig {
            destination_path: "s3://autharie-backups/org-1/source".to_string(),
            server_name: "source-db".to_string(),
            backup_id: None,
        });
        instance
    }

    #[test]
    fn a_cluster_waits_for_the_pods_to_go_before_it_hibernates() {
        let instance = stopped(IdentityProvider::Ferriskey);

        let manifest = cnpg_cluster_manifest(&instance, "default", None, false).unwrap();
        assert_eq!(hibernation_of(&manifest), Some("off"));

        let manifest = cnpg_cluster_manifest(&instance, "default", None, true).unwrap();
        assert_eq!(hibernation_of(&manifest), Some("on"));
    }

    #[test]
    fn a_started_instance_says_off_rather_than_nothing() {
        let manifest = cnpg_cluster_manifest(&instance(), "default", None, false).unwrap();

        assert_eq!(hibernation_of(&manifest), Some("off"));
    }

    #[test]
    fn hibernating_a_cluster_changes_its_annotation_and_nothing_else() {
        let started = recovering(instance());
        let mut halted = recovering(instance());
        halted.spec.stopped = true;

        let running = cnpg_cluster_manifest(&started, "default", None, false).unwrap();
        let hibernated = cnpg_cluster_manifest(&halted, "default", None, true).unwrap();

        assert_eq!(hibernation_of(&running), Some("off"));
        assert_eq!(hibernation_of(&hibernated), Some("on"));
        assert_eq!(running["spec"], hibernated["spec"]);
        assert_eq!(hibernated["spec"]["storage"]["size"], "10Gi");
        assert_eq!(hibernated["spec"]["instances"], 1);
        assert!(hibernated["spec"]["backup"].is_object());
        assert!(hibernated["spec"]["bootstrap"]["recovery"].is_object());
        assert_eq!(running["metadata"]["name"], hibernated["metadata"]["name"]);
    }

    #[test]
    fn a_recovery_without_an_archive_is_refused_before_anything_is_applied() {
        let mut instance = recovering(instance());
        instance.spec.backup = None;

        assert!(cnpg_cluster_manifest(&instance, "default", None, false).is_err());
    }

    fn with_pods(mut deployment: Deployment, pods: i32) -> Deployment {
        deployment.metadata.generation = Some(2);
        deployment.status = Some(k8s_openapi::api::apps::v1::DeploymentStatus {
            observed_generation: Some(2),
            replicas: Some(pods),
            ready_replicas: Some(pods),
            available_replicas: Some(pods),
            ..Default::default()
        });
        deployment
    }

    fn job_with(conditions: &[&str], succeeded: Option<i32>) -> Job {
        use k8s_openapi::api::batch::v1::{JobCondition, JobStatus};

        Job {
            status: Some(JobStatus {
                succeeded,
                conditions: Some(
                    conditions
                        .iter()
                        .map(|kind| JobCondition {
                            type_: (*kind).to_string(),
                            status: "True".to_string(),
                            ..Default::default()
                        })
                        .collect(),
                ),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn drained() -> Vec<Deployment> {
        vec![
            with_pods(api_deployment_with(0), 0),
            with_pods(api_deployment_with(0), 0),
        ]
    }

    #[test]
    fn hibernation_is_withheld_while_pods_remain() {
        let instance = stopped(IdentityProvider::Ferriskey);
        let deployments = vec![
            with_pods(api_deployment_with(0), 0),
            with_pods(api_deployment_with(0), 2),
        ];

        assert!(!hibernation_due(&instance, &deployments, &[], now()));
    }

    #[test]
    fn hibernation_is_withheld_until_the_scale_down_has_been_seen() {
        let instance = stopped(IdentityProvider::Ferriskey);
        let mut unseen = with_pods(api_deployment_with(0), 0);
        unseen.metadata.generation = Some(3);

        assert!(!hibernation_due(&instance, &[unseen], &[], now()));
    }

    #[test]
    fn hibernation_is_withheld_while_a_migration_runs() {
        let instance = stopped(IdentityProvider::Ferriskey);
        let just_created = Job::default();
        let running = job_with(&[], None);

        assert!(!hibernation_due(
            &instance,
            &drained(),
            &[just_created],
            now()
        ));
        assert!(!hibernation_due(&instance, &drained(), &[running], now()));
    }

    #[test]
    fn hibernation_comes_once_the_pods_and_the_migrations_are_gone() {
        let instance = stopped(IdentityProvider::Ferriskey);
        let jobs = [
            job_with(&["Complete"], Some(1)),
            job_with(&["Failed"], None),
        ];

        assert!(hibernation_due(&instance, &drained(), &jobs, now()));
        assert!(hibernation_due(&instance, &[], &[], now()));
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn created(mut job: Job, minutes_ago: i64) -> Job {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

        job.metadata.creation_timestamp =
            Some(Time(now() - chrono::Duration::minutes(minutes_ago)));
        job
    }

    #[test]
    fn a_migration_that_never_ends_stops_holding_hibernation_back_after_the_wait() {
        let instance = stopped(IdentityProvider::Ferriskey);
        let stuck = created(job_with(&[], None), 31);
        let recent = created(job_with(&[], None), 1);

        assert!(hibernation_due(&instance, &drained(), &[stuck], now()));
        assert!(!hibernation_due(&instance, &drained(), &[recent], now()));
    }

    #[test]
    fn a_stuck_migration_does_not_keep_a_stopped_instance_waiting_to_be_ready() {
        let stuck = created(Job::default(), 31);
        let recent = created(Job::default(), 1);

        assert!(!migration_holds_hibernation(&stuck, now()));
        assert!(migration_holds_hibernation(&recent, now()));
        assert!(migration_running(&stuck));
    }

    #[test]
    fn terminating_pods_are_pods_that_remain() {
        let mut leaving = with_pods(api_deployment_with(0), 0);
        leaving.status.as_mut().unwrap().terminating_replicas = Some(1);
        let mut gone = with_pods(api_deployment_with(0), 0);
        gone.status.as_mut().unwrap().terminating_replicas = Some(0);

        assert!(!replicas_ready(&leaving, 0));
        assert!(replicas_ready(&gone, 0));
        assert!(replicas_ready(&with_pods(api_deployment_with(0), 0), 0));
    }

    #[test]
    fn a_running_instance_never_hibernates() {
        let mut instance = stopped(IdentityProvider::Ferriskey);
        instance.spec.stopped = false;

        assert!(!hibernation_due(&instance, &drained(), &[], now()));
    }

    #[test]
    fn a_workload_scaled_to_nothing_is_ready_once_its_pods_are_gone() {
        assert!(!replicas_ready(&with_pods(api_deployment_with(0), 1), 0));
        assert!(replicas_ready(&with_pods(api_deployment_with(0), 0), 0));
    }

    fn with_phase(mut instance: IdentityInstance, phase: Option<Phase>) -> IdentityInstance {
        instance.status = Some(
            autharie_crds::v1alpha::identity_instance::IdentityInstanceStatus {
                phase,
                ..Default::default()
            },
        );
        instance
    }

    #[test]
    fn a_failed_migration_is_cleared_on_the_pass_that_restarts_the_instance() {
        let restarted = with_phase(instance(), Some(Phase::Stopped));

        assert!(restarting(&restarted));
        assert!(migration_failed(&job_with(&["Failed"], None)));
    }

    #[test]
    fn a_failed_migration_of_an_instance_never_stopped_is_left_failed() {
        assert!(migration_failed(&job_with(&["Failed"], None)));
        for phase in [
            None,
            Some(Phase::DatabaseProvisioning),
            Some(Phase::Deploying),
            Some(Phase::Running),
            Some(Phase::Upgrading),
            Some(Phase::Failed),
        ] {
            assert!(!restarting(&with_phase(instance(), phase)));
        }
        assert!(!restarting(&instance()));
    }

    #[test]
    fn an_instance_still_stopped_is_not_restarting() {
        let still_stopped = with_phase(stopped(IdentityProvider::Ferriskey), Some(Phase::Stopped));

        assert!(!restarting(&still_stopped));
    }

    #[test]
    fn only_a_migration_that_ended_failed_is_failed() {
        assert!(!migration_failed(&job_with(&["Complete"], Some(1))));
        assert!(!migration_failed(&job_with(&[], None)));
        assert!(!migration_failed(&Job::default()));
    }

    #[test]
    fn readiness_waits_for_every_replica() {
        let deployment = api_deployment_with(2);

        assert!(!replicas_ready(&with_status(deployment.clone(), 1), 2));
        assert!(replicas_ready(&with_status(deployment, 2), 2));
    }

    #[test]
    fn readiness_with_the_default_replica_count_is_unchanged() {
        let deployment = api_deployment_with(1);

        assert!(!replicas_ready(&with_status(deployment.clone(), 0), 1));
        assert!(replicas_ready(&with_status(deployment, 1), 1));
    }

    #[test]
    fn the_database_instances_of_the_instance_reach_the_cluster_spec() {
        let mut instance = instance();
        instance.spec.database.managed_cluster.instances = 2;

        let spec = cnpg_base_spec(&instance.spec.database.managed_cluster);

        assert_eq!(spec["instances"], json!(2));
        assert_eq!(
            cnpg_base_spec(&self::instance().spec.database.managed_cluster)["instances"],
            json!(1)
        );
    }

    /// Absent is the default and a legitimate one -- an installation that
    /// has not turned tracing on must provision FerrisKey exactly as it
    /// always has, with neither var present at all.
    #[test]
    fn build_ferriskey_api_deployment_omits_observability_vars_when_no_endpoint_is_given() {
        let deployment = build_ferriskey_api_deployment(
            "instance-1-api",
            "default",
            &BTreeMap::new(),
            "ghcr.io/ferriskey/ferriskey-api:1.0.0",
            "instance-1-db-credentials",
            "instance-1-admin",
            "instance-1-db.default.svc",
            "https://auth.acme.test",
            "https://auth.acme.test",
            None,
            1,
            None,
        )
        .unwrap();

        let pod_spec = deployment.spec.unwrap().template.spec.unwrap();
        let container = &pod_spec.containers[0];

        assert!(env_value(container, "ACTIVE_OBSERVABILITY").is_none());
        assert!(env_value(container, "OTLP_ENDPOINT").is_none());
        assert!(env_value(container, "METRICS_ENDPOINT").is_none());
    }

    #[test]
    fn a_shared_instance_connects_to_its_own_database_name() {
        let mut deployment = build_ferriskey_api_deployment(
            "instance-1-api",
            "default",
            &BTreeMap::new(),
            "ghcr.io/ferriskey/ferriskey-api:1.0.0",
            "instance-1-db-credentials",
            "instance-1-admin",
            "free-db-rw.system.svc",
            "https://auth.acme.test",
            "https://auth.acme.test",
            None,
            1,
            None,
        )
        .unwrap();

        use_database_name(&mut deployment, "free_abc");

        let pod_spec = deployment.spec.unwrap().template.spec.unwrap();
        assert_eq!(
            env_value(&pod_spec.containers[0], "DATABASE_NAME")
                .and_then(|var| var.value.as_deref()),
            Some("free_abc")
        );
    }

    #[test]
    fn a_shared_instance_is_replaced_rather_than_rolled() {
        let instance = stopped(IdentityProvider::Ferriskey);
        let dedicated =
            build_ferriskey_runtime(&instance, "default", None, None, None, None).unwrap();
        assert!(dedicated.api.spec.as_ref().unwrap().strategy.is_none());

        let mut api = dedicated.api;
        assert!(!is_recreated(&api));
        replace_rather_than_roll(&mut api);
        assert!(is_recreated(&api));
        assert_eq!(
            api.spec
                .unwrap()
                .strategy
                .and_then(|strategy| strategy.type_)
                .as_deref(),
            Some("Recreate")
        );
    }

    /// The acceptance criterion itself, including the part only a running
    /// FerrisKey exposed: it refuses to start with `ACTIVE_OBSERVABILITY`
    /// set but no `METRICS_ENDPOINT`, so all three vars must land together.
    #[test]
    fn build_ferriskey_api_deployment_activates_observability_when_an_endpoint_is_given() {
        let deployment = build_ferriskey_api_deployment(
            "instance-1-api",
            "default",
            &BTreeMap::new(),
            "ghcr.io/ferriskey/ferriskey-api:1.0.0",
            "instance-1-db-credentials",
            "instance-1-admin",
            "instance-1-db.default.svc",
            "https://auth.acme.test",
            "https://auth.acme.test",
            Some("http://release-herald.dataplane.svc:4318"),
            1,
            None,
        )
        .unwrap();

        let pod_spec = deployment.spec.unwrap().template.spec.unwrap();
        let container = &pod_spec.containers[0];

        assert_eq!(
            env_value(container, "ACTIVE_OBSERVABILITY").and_then(|env| env.value.as_deref()),
            Some("true")
        );
        assert_eq!(
            env_value(container, "OTLP_ENDPOINT").and_then(|env| env.value.as_deref()),
            Some("http://release-herald.dataplane.svc:4318")
        );
        assert_eq!(
            env_value(container, "METRICS_ENDPOINT").and_then(|env| env.value.as_deref()),
            Some("http://release-herald.dataplane.svc:4318")
        );
    }

    /// #297: the fallback used to be an address only the *cluster* could
    /// resolve (the API's own Service DNS name) or a production domain no
    /// local instance was ever given -- so a browser sent there got exactly
    /// the "server not found" this closes. `spec.hostname` is what the
    /// `HTTPRoute` actually serves the instance on, and is required on the
    /// CRD, so it is always there to fall back to.
    #[test]
    fn ferriskey_webapp_url_uses_override_or_falls_back_to_the_instance_hostname() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;
        assert_eq!(
            ferriskey_webapp_url(&instance, None),
            "https://auth.acme.test"
        );

        instance.spec.ferriskey = Some(FerriskeyConfig {
            webapp_url: Some("http://localhost:5555".to_string()),
            api_base_url: None,
            pooled_cell: false,
        });
        assert_eq!(
            ferriskey_webapp_url(&instance, None),
            "http://localhost:5555"
        );

        instance.spec.ferriskey = Some(FerriskeyConfig {
            webapp_url: Some("   ".to_string()),
            api_base_url: None,
            pooled_cell: false,
        });
        assert_eq!(
            ferriskey_webapp_url(&instance, None),
            "https://auth.acme.test"
        );
    }

    /// #298: a local Gateway whose HTTPS listener is not actually reachable
    /// on 443 (k3d remaps it to a host port) needs that port spelled out, or
    /// a browser's redirect assumes 443 and cannot connect -- the exact
    /// "server not found" this closes.
    #[test]
    fn ferriskey_webapp_url_carries_an_explicit_public_port_when_one_is_given() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;

        assert_eq!(
            ferriskey_webapp_url(&instance, Some(8444)),
            "https://auth.acme.test:8444"
        );
    }

    #[test]
    fn ferriskey_api_base_url_uses_override_or_falls_back_to_the_instance_hostname() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;
        assert_eq!(
            ferriskey_api_base_url(&instance, None),
            "https://auth.acme.test/api"
        );

        instance.spec.ferriskey = Some(FerriskeyConfig {
            webapp_url: None,
            api_base_url: Some("http://localhost:3333/api".to_string()),
            pooled_cell: false,
        });
        assert_eq!(
            ferriskey_api_base_url(&instance, None),
            "http://localhost:3333/api"
        );

        instance.spec.ferriskey = Some(FerriskeyConfig {
            webapp_url: None,
            api_base_url: Some(" ".to_string()),
            pooled_cell: false,
        });
        assert_eq!(
            ferriskey_api_base_url(&instance, None),
            "https://auth.acme.test/api"
        );
    }

    fn env_of(deployment: &Deployment) -> Vec<(String, Option<String>)> {
        deployment
            .spec
            .iter()
            .filter_map(|spec| spec.template.spec.as_ref())
            .flat_map(|pod| pod.containers.iter())
            .flat_map(|container| container.env.iter().flatten())
            .map(|env| (env.name.clone(), env.value.clone()))
            .collect()
    }

    #[test]
    fn the_webapp_of_a_pooled_cell_calls_a_relative_api_whatever_its_hostname() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;
        instance.spec.ferriskey = Some(FerriskeyConfig {
            webapp_url: None,
            api_base_url: Some("http://localhost:3333/api".to_string()),
            pooled_cell: true,
        });

        assert!(is_pooled_cell(&instance));
        assert_eq!(ferriskey_api_base_url(&instance, Some(8444)), "/api");
    }

    #[test]
    fn a_pooled_cell_allows_every_host_of_its_zone_or_its_own_host_when_it_has_no_zone() {
        assert_eq!(
            webapp_allowed_hosts("cell-1a2b3c4d.cells.autharie.fr"),
            "*.cells.autharie.fr"
        );
        assert_eq!(
            webapp_allowed_hosts("cell-1a2b3c4d.autharie.local"),
            "*.autharie.local"
        );
        assert_eq!(webapp_allowed_hosts("localhost"), "localhost");
        assert_eq!(webapp_allowed_hosts("cell."), "cell.");
    }

    #[test]
    fn only_the_api_of_a_pooled_cell_derives_its_webapp_url_from_the_request() {
        let flag = (
            WEBAPP_URL_FROM_REQUEST.to_string(),
            Some("true".to_string()),
        );
        let ordinary = api_deployment_with(2);
        let hosts = (
            WEBAPP_ALLOWED_HOSTS.to_string(),
            Some("*.cells.autharie.fr".to_string()),
        );
        let mut cell = api_deployment_with(2);
        derive_webapp_url_from_request(&mut cell, "cell-1a2b3c4d.cells.autharie.fr");

        assert!(!env_of(&ordinary).contains(&flag));
        assert!(
            !env_of(&ordinary)
                .iter()
                .any(|(name, _)| name == WEBAPP_ALLOWED_HOSTS)
        );
        assert!(env_of(&cell).contains(&flag));
        assert!(env_of(&cell).contains(&hosts));

        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;
        assert!(!is_pooled_cell(&instance));
    }

    #[test]
    fn ferriskey_api_base_url_carries_an_explicit_public_port_when_one_is_given() {
        let mut instance = instance();
        instance.spec.provider = IdentityProvider::Ferriskey;

        assert_eq!(
            ferriskey_api_base_url(&instance, Some(8444)),
            "https://auth.acme.test:8444/api"
        );
    }

    fn upgrade_at(phase: Option<Phase>) -> IdentityInstanceUpgrade {
        let mut upgrade = IdentityInstanceUpgrade::new(
            "upgrade-x",
            autharie_crds::v1alpha::identity_instance_upgrade::IdentityInstanceUpgradeSpec {
                identity_instance_ref:
                    autharie_crds::v1alpha::identity_instance_upgrade::IdentityInstanceRef {
                        name: "deployment-x".to_string(),
                    },
                target_version: "26.0.1".to_string(),
                strategy: Default::default(),
                approved: true,
            },
        );
        upgrade.status = Some(
            autharie_crds::v1alpha::identity_instance_upgrade::IdentityInstanceUpgradeStatus {
                phase,
                ..Default::default()
            },
        );
        upgrade
    }

    /// The half that was missing. An upgrade the deadline gave up on is over,
    /// and counting it as still running is what had the instance re-asserting
    /// `Upgrading` every fifteen seconds for ever afterwards.
    #[test]
    fn a_failed_upgrade_is_over() {
        assert!(upgrade_is_over(&upgrade_at(Some(Phase::Failed))));
    }

    #[test]
    fn a_finished_upgrade_is_over() {
        assert!(upgrade_is_over(&upgrade_at(Some(Phase::Running))));
    }

    /// Everything else is still on its way, including an upgrade that has not
    /// written a status yet.
    #[test]
    fn an_upgrade_still_moving_is_not_over() {
        for phase in [
            Some(Phase::Pending),
            Some(Phase::Updating),
            Some(Phase::Upgrading),
            None,
        ] {
            assert!(
                !upgrade_is_over(&upgrade_at(phase.clone())),
                "{phase:?} is not over"
            );
        }
    }
}
