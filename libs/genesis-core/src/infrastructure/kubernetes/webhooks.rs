use std::collections::BTreeMap;

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, WebhookRequest, WebhookRequestKind as CrdRequestKind, WebhookRequestStatus,
    WebhookSpec,
};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Patch, PatchParams};
use kube::core::ObjectMeta;
use kube::{Api, Client, Resource};
use serde_json::json;
use tracing::info;

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::webhooks_payload::{
    WebhookDefinition, WebhookRequestEntry, WebhookRequestKind, WebhooksPayloadV1,
};
use crate::domain::error::GenesisError;
use crate::domain::ports::WebhookPort;

pub const WEBHOOKS_FIELD_MANAGER: &str = "genesis-webhooks";

pub struct KubeWebhookPort {
    client: Client,
}

impl KubeWebhookPort {
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

impl WebhookPort for KubeWebhookPort {
    async fn revision<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
    ) -> Result<u64, GenesisError> {
        Ok(self.instance(reference).await?.spec.webhooks_revision)
    }

    async fn apply<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        desired: &'a WebhooksPayloadV1,
    ) -> Result<(), GenesisError> {
        let instance = self.instance(reference).await?;
        let owner = instance
            .controller_owner_ref(&())
            .ok_or_else(|| GenesisError::Kubernetes {
                message: format!("IdentityInstance {} has no uid", reference.name),
            })?;

        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &reference.namespace);
        let secret = headers_secret(reference, desired, owner);

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            "writing the webhook headers Secret"
        );

        secrets
            .patch(
                &headers_secret_name(reference),
                &PatchParams::apply(WEBHOOKS_FIELD_MANAGER).force(),
                &Patch::Apply(&secret),
            )
            .await
            .map_err(|error| kubernetes_error("write the webhook headers Secret", &error))?;

        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), &reference.namespace);

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            "setting the webhooks"
        );

        api.patch(
            &reference.name,
            &PatchParams {
                field_manager: Some(WEBHOOKS_FIELD_MANAGER.to_string()),
                ..PatchParams::default()
            },
            &Patch::Merge(&webhooks_patch(desired)),
        )
        .await
        .map_err(|error| kubernetes_error("set the webhooks", &error))?;

        Ok(())
    }

    async fn request<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        request: &'a WebhookRequestEntry,
    ) -> Result<(), GenesisError> {
        let instance = self.instance(reference).await?;
        let status = instance
            .status
            .as_ref()
            .map(|status| status.webhook_requests.as_slice())
            .unwrap_or_default();

        let Some(requests) = with_request(&instance.spec.webhook_requests, status, request) else {
            return Ok(());
        };

        patch_requests(&self.client, reference, &instance, &requests).await
    }
}

pub async fn patch_requests(
    client: &Client,
    reference: &IdentityInstanceRef,
    instance: &IdentityInstance,
    requests: &[WebhookRequest],
) -> Result<(), GenesisError> {
    let api: Api<IdentityInstance> = Api::namespaced(client.clone(), &reference.namespace);

    api.patch(
        &reference.name,
        &PatchParams {
            field_manager: Some(WEBHOOKS_FIELD_MANAGER.to_string()),
            ..PatchParams::default()
        },
        &Patch::Merge(&requests_patch(
            instance.metadata.resource_version.as_deref(),
            requests,
        )),
    )
    .await
    .map_err(|error| kubernetes_error("set the webhook requests", &error))?;

    Ok(())
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

pub fn headers_secret_name(reference: &IdentityInstanceRef) -> String {
    format!("{}-webhook-headers", reference.name)
}

pub fn secrets_secret_name(instance: &str) -> String {
    format!("{instance}-webhook-secrets")
}

fn headers_secret(
    reference: &IdentityInstanceRef,
    desired: &WebhooksPayloadV1,
    owner: k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference,
) -> Secret {
    let data: BTreeMap<String, ByteString> = desired
        .headers
        .secret_data()
        .into_iter()
        .map(|(key, value)| (key, ByteString(value.into_bytes())))
        .collect();

    Secret {
        metadata: ObjectMeta {
            name: Some(headers_secret_name(reference)),
            namespace: Some(reference.namespace.clone()),
            owner_references: Some(vec![owner]),
            ..Default::default()
        },
        type_: Some("Opaque".to_string()),
        data: Some(data),
        ..Default::default()
    }
}

fn to_spec(definition: &WebhookDefinition) -> WebhookSpec {
    WebhookSpec {
        id: definition.id.to_string(),
        realm: definition.realm.clone(),
        name: definition.name.clone(),
        description: definition.description.clone(),
        endpoint: definition.endpoint.clone(),
        events: definition.events.clone(),
        header_names: definition.header_names.clone(),
    }
}

pub fn webhooks_patch(desired: &WebhooksPayloadV1) -> serde_json::Value {
    let webhooks: Vec<WebhookSpec> = desired.webhooks.iter().map(to_spec).collect();

    json!({
        "spec": {
            "webhooks": webhooks,
            "webhooksRevision": desired.revision,
        }
    })
}

fn requests_patch(
    resource_version: Option<&str>,
    requests: &[WebhookRequest],
) -> serde_json::Value {
    json!({
        "metadata": { "resourceVersion": resource_version },
        "spec": { "webhookRequests": requests },
    })
}

fn to_request(entry: &WebhookRequestEntry) -> WebhookRequest {
    WebhookRequest {
        id: entry.id.clone(),
        kind: match entry.kind {
            WebhookRequestKind::RotateSecret => CrdRequestKind::RotateSecret,
            WebhookRequestKind::RetryDelivery => CrdRequestKind::RetryDelivery,
        },
        webhook_id: entry.webhook_id.clone(),
        delivery_id: entry.delivery_id.clone(),
    }
}

pub fn with_request(
    pending: &[WebhookRequest],
    executed: &[WebhookRequestStatus],
    entry: &WebhookRequestEntry,
) -> Option<Vec<WebhookRequest>> {
    let known = pending.iter().any(|request| request.id == entry.id)
        || executed.iter().any(|status| status.id == entry.id);
    if known {
        return None;
    }

    let mut requests = pending.to_vec();
    requests.push(to_request(entry));
    Some(requests)
}

pub fn without_requests(
    pending: &[WebhookRequest],
    published: &[&str],
) -> Option<Vec<WebhookRequest>> {
    let kept: Vec<WebhookRequest> = pending
        .iter()
        .filter(|request| !published.contains(&request.id.as_str()))
        .cloned()
        .collect();

    (kept.len() != pending.len()).then_some(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::webhooks_payload::WebhooksPayloadV1;
    use autharie_crds::v1alpha::identity_instance::WebhookRequestPhase;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
    use serde_json::{Value, json};
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const HOOK: Uuid = Uuid::from_u128(2);
    const VALUE: &str = "Bearer s3cr3t-value";

    fn reference() -> IdentityInstanceRef {
        IdentityInstanceRef::for_deployment(DEPLOYMENT, "tenant-a")
    }

    fn desired() -> WebhooksPayloadV1 {
        WebhooksPayloadV1::from_value(&json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 7,
            "webhooks": [{
                "id": HOOK,
                "realm": "master",
                "name": "audit",
                "description": "d",
                "endpoint": "https://hooks.example.com",
                "events": ["user.created"],
                "header_names": ["Authorization"],
            }],
            "headers": { HOOK.to_string(): { "Authorization": VALUE } },
        }))
        .expect("valid")
    }

    fn entry(id: &str) -> WebhookRequestEntry {
        WebhookRequestEntry {
            id: id.to_string(),
            kind: WebhookRequestKind::RotateSecret,
            webhook_id: HOOK.to_string(),
            delivery_id: None,
        }
    }

    fn pending(id: &str) -> WebhookRequest {
        to_request(&entry(id))
    }

    #[test]
    fn the_patch_carries_the_spec_fields_in_camel_case_and_no_header_value() {
        let patch = webhooks_patch(&desired());

        assert_eq!(patch["spec"]["webhooksRevision"], json!(7));
        assert_eq!(
            patch["spec"]["webhooks"][0],
            json!({
                "id": HOOK.to_string(),
                "realm": "master",
                "name": "audit",
                "description": "d",
                "endpoint": "https://hooks.example.com",
                "events": ["user.created"],
                "headerNames": ["Authorization"],
            })
        );
        assert!(!patch.to_string().contains(VALUE));
    }

    #[test]
    fn an_empty_set_is_written_as_an_empty_list_so_it_clears_the_previous_one() {
        let empty = WebhooksPayloadV1::from_value(&json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 8,
            "webhooks": [],
        }))
        .expect("valid");

        assert_eq!(webhooks_patch(&empty)["spec"]["webhooks"], json!([]));
    }

    #[test]
    fn the_headers_secret_is_named_after_the_instance_and_owned_by_it() {
        let owner = OwnerReference {
            api_version: "autharie.fr/v1alpha".to_string(),
            kind: "IdentityInstance".to_string(),
            name: reference().name,
            uid: "uid-1".to_string(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        };

        let secret = headers_secret(&reference(), &desired(), owner);

        assert_eq!(
            secret.metadata.name.as_deref(),
            Some(format!("deployment-{DEPLOYMENT}-webhook-headers").as_str())
        );
        assert_eq!(secret.metadata.namespace.as_deref(), Some("tenant-a"));
        assert_eq!(
            secret.metadata.owner_references.as_ref().expect("owned")[0].uid,
            "uid-1"
        );
        let data = secret.data.expect("data");
        assert_eq!(data.len(), 1);
        assert_eq!(
            data[&format!("{HOOK}.Authorization")].0,
            VALUE.as_bytes().to_vec()
        );
    }

    #[test]
    fn removed_headers_leave_the_secret_data_because_it_is_replaced_whole() {
        let none = WebhooksPayloadV1::from_value(&json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": 9,
            "webhooks": [],
        }))
        .expect("valid");
        let owner = OwnerReference::default();

        let secret = headers_secret(&reference(), &none, owner);

        assert!(secret.data.expect("data present, empty").is_empty());
    }

    #[test]
    fn the_secrets_secret_is_named_after_the_instance() {
        assert_eq!(
            secrets_secret_name("deployment-x"),
            "deployment-x-webhook-secrets"
        );
    }

    #[test]
    fn a_new_request_is_appended_after_the_pending_ones() {
        let requests = with_request(&[pending("a")], &[], &entry("b")).expect("appended");

        let ids: Vec<&str> = requests.iter().map(|request| request.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn a_request_already_pending_is_not_added_again() {
        assert_eq!(with_request(&[pending("a")], &[], &entry("a")), None);
    }

    #[test]
    fn a_request_the_operator_already_executed_is_not_added_again() {
        let executed = [WebhookRequestStatus {
            id: "a".to_string(),
            phase: WebhookRequestPhase::Done,
            message: None,
        }];

        assert_eq!(with_request(&[], &executed, &entry("a")), None);
    }

    #[test]
    fn a_retry_keeps_its_delivery() {
        let retry = WebhookRequestEntry {
            kind: WebhookRequestKind::RetryDelivery,
            delivery_id: Some("d-1".to_string()),
            ..entry("r")
        };

        let requests = with_request(&[], &[], &retry).expect("appended");

        assert_eq!(requests[0].kind, CrdRequestKind::RetryDelivery);
        assert_eq!(requests[0].delivery_id.as_deref(), Some("d-1"));
    }

    #[test]
    fn published_requests_are_pruned_and_the_rest_stay() {
        let kept = without_requests(&[pending("a"), pending("b")], &["a"]).expect("pruned");

        assert_eq!(kept, vec![pending("b")]);
    }

    #[test]
    fn nothing_to_prune_means_no_write() {
        assert_eq!(without_requests(&[pending("a")], &["z"]), None);
        assert_eq!(without_requests(&[], &["a"]), None);
    }

    #[test]
    fn the_requests_patch_pins_the_resource_version_so_a_concurrent_write_conflicts() {
        let patch = requests_patch(Some("42"), &[pending("a")]);

        assert_eq!(patch["metadata"]["resourceVersion"], json!("42"));
        assert_eq!(patch["spec"]["webhookRequests"][0]["id"], json!("a"));
        assert_eq!(
            patch["spec"]["webhookRequests"][0]["kind"],
            Value::String("rotateSecret".to_string())
        );
    }

    #[test]
    fn the_webhooks_have_a_field_manager_of_their_own() {
        assert_ne!(WEBHOOKS_FIELD_MANAGER, "genesis");
    }
}
