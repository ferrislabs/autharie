use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, Client, Resource};
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use crate::domain::OperatorError;
use crate::domain::webhooks::WebhookError;
use crate::domain::webhooks::ports::{
    SecretsSnapshot, WebhookDeliveriesStore, WebhookSecretsPort, WebhookStatusReport,
    WebhookStatusRepository,
};

use super::ferriskey_admin::name_and_namespace;

/// Header values, one key per `<webhook id>.<header name>`. Genesis writes it.
pub fn webhook_headers_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-webhook-headers")
}

/// Signing secrets, one key per webhook id. The operator writes it.
pub fn webhook_secrets_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-webhook-secrets")
}

/// A fingerprint per webhook id of the header values last sent, because
/// FerrisKey never gives them back. The operator's own bookkeeping.
pub fn webhook_applied_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-webhook-applied")
}

pub struct KubeWebhookSecrets {
    client: Client,
}

impl KubeWebhookSecrets {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn api(&self, namespace: &str) -> Api<Secret> {
        Api::namespaced(self.client.clone(), namespace)
    }

    async fn read(
        &self,
        secrets: &Api<Secret>,
        name: &str,
    ) -> Result<BTreeMap<String, String>, WebhookError> {
        let secret = secrets.get_opt(name).await.map_err(kube_error)?;
        Ok(secret
            .and_then(|secret| secret.data)
            .map(|data| {
                data.into_iter()
                    .filter_map(|(key, value)| Some((key, String::from_utf8(value.0).ok()?)))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn set(
        &self,
        instance: &IdentityInstance,
        secret_name: &str,
        key: &str,
        value: &str,
    ) -> Result<(), WebhookError> {
        let (_, namespace) = located(instance)?;
        let secrets = self.api(namespace);
        let patch = json!({ "stringData": { key: value } });

        match secrets
            .patch(secret_name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => {
                let secret = Secret {
                    metadata: ObjectMeta {
                        name: Some(secret_name.to_string()),
                        namespace: Some(namespace.to_string()),
                        owner_references: instance
                            .controller_owner_ref(&())
                            .map(|owner| vec![owner]),
                        ..Default::default()
                    },
                    string_data: Some(BTreeMap::from([(key.to_string(), value.to_string())])),
                    ..Default::default()
                };
                secrets
                    .create(&PostParams::default(), &secret)
                    .await
                    .map(|_| ())
                    .map_err(kube_error)
            }
            Err(error) => Err(kube_error(error)),
        }
    }
}

fn located(instance: &IdentityInstance) -> Result<(&str, &str), WebhookError> {
    name_and_namespace(instance).ok_or_else(|| WebhookError::Retry {
        message: "the instance has no name or namespace yet".to_string(),
    })
}

fn is_not_found(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(response) if response.code == 404)
}

/// The message names no key and no value: a Kubernetes error may quote the
/// request, and the request holds a secret.
fn kube_error(error: kube::Error) -> WebhookError {
    let reason = match &error {
        kube::Error::Api(response) => format!("status {}", response.code),
        _ => "no answer".to_string(),
    };
    WebhookError::Retry {
        message: format!("the cluster could not read or write a webhook secret ({reason})"),
    }
}

/// Keys set to null are removed by a merge patch.
fn removal_patch(ids: &[String]) -> Value {
    let removed: serde_json::Map<String, Value> =
        ids.iter().map(|id| (id.clone(), Value::Null)).collect();
    json!({ "data": removed })
}

impl WebhookSecretsPort for KubeWebhookSecrets {
    async fn load(&self, instance: &IdentityInstance) -> Result<SecretsSnapshot, WebhookError> {
        let (name, namespace) = located(instance)?;
        let secrets = self.api(namespace);

        let headers = self
            .read(&secrets, &webhook_headers_secret_name(name))
            .await?;
        let applied = self
            .read(&secrets, &webhook_applied_secret_name(name))
            .await?;
        let stored = self
            .read(&secrets, &webhook_secrets_secret_name(name))
            .await?;

        Ok(SecretsSnapshot {
            headers,
            applied,
            secret_ids: stored.into_keys().collect::<BTreeSet<_>>(),
        })
    }

    async fn put_secret(
        &self,
        instance: &IdentityInstance,
        webhook_id: &str,
        secret: &str,
    ) -> Result<(), WebhookError> {
        let (name, _) = located(instance)?;
        self.set(
            instance,
            &webhook_secrets_secret_name(name),
            webhook_id,
            secret,
        )
        .await
    }

    async fn put_applied(
        &self,
        instance: &IdentityInstance,
        webhook_id: &str,
        fingerprint: &str,
    ) -> Result<(), WebhookError> {
        let (name, _) = located(instance)?;
        self.set(
            instance,
            &webhook_applied_secret_name(name),
            webhook_id,
            fingerprint,
        )
        .await
    }

    async fn forget(
        &self,
        instance: &IdentityInstance,
        webhook_ids: &[String],
    ) -> Result<(), WebhookError> {
        let (name, namespace) = located(instance)?;
        let secrets = self.api(namespace);
        let patch = removal_patch(webhook_ids);

        for secret_name in [
            webhook_secrets_secret_name(name),
            webhook_applied_secret_name(name),
        ] {
            match secrets
                .patch(&secret_name, &PatchParams::default(), &Patch::Merge(&patch))
                .await
            {
                Ok(_) => {}
                Err(error) if is_not_found(&error) => {}
                Err(error) => return Err(kube_error(error)),
            }
        }
        Ok(())
    }
}

/// The recent deliveries as JSON, for Herald to read. The operator writes it.
pub fn webhook_deliveries_config_map_name(instance_name: &str) -> String {
    format!("{instance_name}-webhook-deliveries")
}

pub const DELIVERIES_KEY: &str = "deliveries.json";

pub struct KubeWebhookDeliveries {
    client: Client,
}

impl KubeWebhookDeliveries {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl WebhookDeliveriesStore for KubeWebhookDeliveries {
    async fn current(&self, instance: &IdentityInstance) -> Result<Option<String>, WebhookError> {
        let (name, namespace) = located(instance)?;
        let maps: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);
        Ok(maps
            .get_opt(&webhook_deliveries_config_map_name(name))
            .await
            .map_err(kube_error)?
            .and_then(|map| map.data)
            .and_then(|mut data| data.remove(DELIVERIES_KEY)))
    }

    async fn publish(
        &self,
        instance: &IdentityInstance,
        document: &str,
    ) -> Result<(), WebhookError> {
        let (name, namespace) = located(instance)?;
        let maps: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);
        let map_name = webhook_deliveries_config_map_name(name);
        let patch = json!({ "data": { DELIVERIES_KEY: document } });

        match maps
            .patch(&map_name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => {
                let map = ConfigMap {
                    metadata: ObjectMeta {
                        name: Some(map_name),
                        namespace: Some(namespace.to_string()),
                        owner_references: instance
                            .controller_owner_ref(&())
                            .map(|owner| vec![owner]),
                        ..Default::default()
                    },
                    data: Some(BTreeMap::from([(
                        DELIVERIES_KEY.to_string(),
                        document.to_string(),
                    )])),
                    ..Default::default()
                };
                maps.create(&PostParams::default(), &map)
                    .await
                    .map(|_| ())
                    .map_err(kube_error)
            }
            Err(error) => Err(kube_error(error)),
        }
    }
}

pub struct KubeWebhookStatus {
    client: Client,
}

impl KubeWebhookStatus {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

/// A merge patch replaces a list whole and leaves a field it does not mention,
/// so each list is always named: an emptied one has to clear what was there.
pub fn webhook_status_patch(report: &WebhookStatusReport) -> Value {
    json!({
        "status": {
            "webhooks": report.webhooks,
            "realms": report.realms,
            "webhookRequests": report.webhook_requests,
        }
    })
}

impl WebhookStatusRepository for KubeWebhookStatus {
    async fn patch_webhook_status(
        &self,
        instance: &IdentityInstance,
        report: &WebhookStatusReport,
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
            &Patch::Merge(&webhook_status_patch(report)),
        )
        .await
        .map(|_| ())
        .map_err(|error| OperatorError::Kube {
            message: error.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use autharie_crds::v1alpha::identity_instance::{
        WebhookPhase, WebhookRequestPhase, WebhookRequestStatus, WebhookStatus,
    };

    use super::*;

    #[test]
    fn the_secrets_are_named_after_the_instance() {
        assert_eq!(webhook_headers_secret_name("acme"), "acme-webhook-headers");
        assert_eq!(webhook_secrets_secret_name("acme"), "acme-webhook-secrets");
        assert_eq!(webhook_applied_secret_name("acme"), "acme-webhook-applied");
        assert_eq!(
            webhook_deliveries_config_map_name("acme"),
            "acme-webhook-deliveries"
        );
    }

    #[test]
    fn an_empty_report_clears_every_list() {
        let patch = webhook_status_patch(&WebhookStatusReport::default());

        assert_eq!(patch["status"]["webhooks"], json!([]));
        assert_eq!(patch["status"]["realms"], json!([]));
        assert_eq!(patch["status"]["webhookRequests"], json!([]));
        assert_eq!(patch["status"].as_object().unwrap().len(), 3);
    }

    #[test]
    fn the_patch_touches_nothing_but_the_webhook_fields() {
        let patch = webhook_status_patch(&WebhookStatusReport {
            webhooks: vec![WebhookStatus {
                id: "w1".to_string(),
                ferriskey_id: Some("f1".to_string()),
                phase: WebhookPhase::Active,
                message: None,
                secret_version: 2,
            }],
            realms: vec!["master".to_string()],
            webhook_requests: vec![WebhookRequestStatus {
                id: "r1".to_string(),
                phase: WebhookRequestPhase::Done,
                message: None,
            }],
        });

        // Sorted: whether serde_json keeps insertion order depends on which
        // crates in the workspace turn on its `preserve_order` feature.
        let mut keys: Vec<_> = patch["status"].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["realms", "webhookRequests", "webhooks"]);
        assert_eq!(patch["status"]["webhooks"][0]["ferriskeyId"], "f1");
        assert_eq!(patch["status"]["webhooks"][0]["secretVersion"], 2);
        assert_eq!(patch["status"]["webhookRequests"][0]["phase"], "Done");
    }

    #[test]
    fn forgetting_names_each_key_as_null() {
        let patch = removal_patch(&["a".to_string(), "b".to_string()]);

        assert_eq!(patch, json!({"data": {"a": null, "b": null}}));
    }

    #[test]
    fn a_cluster_error_never_quotes_the_request() {
        let error = kube_error(kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".to_string(),
            message: "invalid value whsec-supersecret".to_string(),
            reason: "Invalid".to_string(),
            code: 422,
        }));

        assert!(!error.message().contains("whsec"));
        assert!(error.message().contains("422"));
    }
}
