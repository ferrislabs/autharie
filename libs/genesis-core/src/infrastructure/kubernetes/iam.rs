use autharie_crds::v1alpha::identity_instance::{
    IamRequestSpec, IamRequestStatus, IdentityInstance,
};
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::{Map, Value, json};
use tracing::info;

use crate::domain::entities::iam_payload::IamRequestEntry;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::error::GenesisError;
use crate::domain::ports::{IamPort, IamResultsPort};

pub const IAM_FIELD_MANAGER: &str = "genesis-iam";

pub fn iam_results_secret_name(instance: &str) -> String {
    format!("{instance}-iam-results")
}

pub struct KubeIamPort {
    client: Client,
}

impl KubeIamPort {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn instances(&self, reference: &IdentityInstanceRef) -> Api<IdentityInstance> {
        Api::namespaced(self.client.clone(), &reference.namespace)
    }

    async fn patch_requests(
        &self,
        reference: &IdentityInstanceRef,
        instance: &IdentityInstance,
        requests: &[IamRequestSpec],
    ) -> Result<(), GenesisError> {
        self.instances(reference)
            .patch(
                &reference.name,
                &PatchParams {
                    field_manager: Some(IAM_FIELD_MANAGER.to_string()),
                    ..PatchParams::default()
                },
                &Patch::Merge(&requests_patch(
                    instance.metadata.resource_version.as_deref(),
                    requests,
                )),
            )
            .await
            .map_err(|error| kubernetes_error("set the IAM requests", &error))?;

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

impl IamPort for KubeIamPort {
    async fn request<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        request: &'a IamRequestEntry,
    ) -> Result<(), GenesisError> {
        let instance = self
            .instances(reference)
            .get_opt(&reference.name)
            .await
            .map_err(|error| kubernetes_error("read the IdentityInstance", &error))?
            .ok_or_else(|| GenesisError::Kubernetes {
                message: format!("IdentityInstance {} does not exist yet", reference.name),
            })?;

        let Some(requests) = with_request(
            &instance.spec.iam_requests,
            recorded_requests(&instance),
            request,
        ) else {
            return Ok(());
        };

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            request_id = %request.request_id,
            "writing an IAM request"
        );

        self.patch_requests(reference, &instance, &requests).await
    }

    async fn drop_requests<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        request_ids: &'a [&'a str],
    ) -> Result<(), GenesisError> {
        let Some(instance) = self
            .instances(reference)
            .get_opt(&reference.name)
            .await
            .map_err(|error| kubernetes_error("read the IdentityInstance", &error))?
        else {
            return Ok(());
        };

        let Some(kept) = without_requests(&instance.spec.iam_requests, request_ids) else {
            return Ok(());
        };

        self.patch_requests(reference, &instance, &kept).await
    }
}

impl IamResultsPort for KubeIamPort {
    async fn read<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        request_id: &'a str,
    ) -> Result<Option<Value>, GenesisError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &reference.namespace);
        let secret = secrets
            .get_opt(&iam_results_secret_name(&reference.name))
            .await
            .map_err(|error| kubernetes_error("read the IAM results Secret", &error))?;

        let Some(stored) = secret
            .and_then(|secret| secret.data)
            .and_then(|mut data| data.remove(request_id))
        else {
            return Ok(None);
        };

        serde_json::from_slice(&stored.0)
            .map(Some)
            .map_err(|_| GenesisError::Kubernetes {
                message: "the stored IAM result is not JSON".to_string(),
            })
    }

    async fn forget<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        request_ids: &'a [&'a str],
    ) -> Result<(), GenesisError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &reference.namespace);

        match secrets
            .patch(
                &iam_results_secret_name(&reference.name),
                &PatchParams {
                    field_manager: Some(IAM_FIELD_MANAGER.to_string()),
                    ..PatchParams::default()
                },
                &Patch::Merge(&removal_patch(request_ids)),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(response)) if response.code == 404 => Ok(()),
            Err(error) => Err(kubernetes_error("drop IAM results", &error)),
        }
    }
}

fn recorded_requests(instance: &IdentityInstance) -> &[IamRequestStatus] {
    instance
        .status
        .as_ref()
        .and_then(|status| status.iam.as_ref())
        .map(|iam| iam.requests.as_slice())
        .unwrap_or_default()
}

fn requests_patch(resource_version: Option<&str>, requests: &[IamRequestSpec]) -> Value {
    json!({
        "metadata": { "resourceVersion": resource_version },
        "spec": { "iamRequests": requests },
    })
}

fn removal_patch(request_ids: &[&str]) -> Value {
    let removed: Map<String, Value> = request_ids
        .iter()
        .map(|id| ((*id).to_string(), Value::Null))
        .collect();
    json!({ "data": removed })
}

fn to_spec(entry: &IamRequestEntry) -> IamRequestSpec {
    IamRequestSpec {
        request_id: entry.request_id.clone(),
        realm: entry.realm.clone(),
        operation: entry.operation.clone(),
        sealing_key: entry.sealing_key.clone(),
        expires_at: entry
            .expires_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

pub fn with_request(
    pending: &[IamRequestSpec],
    executed: &[IamRequestStatus],
    entry: &IamRequestEntry,
) -> Option<Vec<IamRequestSpec>> {
    let known = pending
        .iter()
        .any(|request| request.request_id == entry.request_id)
        || executed
            .iter()
            .any(|status| status.request_id == entry.request_id);
    if known {
        return None;
    }

    let mut requests = pending.to_vec();
    requests.push(to_spec(entry));
    Some(requests)
}

pub fn without_requests(
    pending: &[IamRequestSpec],
    published: &[&str],
) -> Option<Vec<IamRequestSpec>> {
    let kept: Vec<IamRequestSpec> = pending
        .iter()
        .filter(|request| !published.contains(&request.request_id.as_str()))
        .cloned()
        .collect();

    (kept.len() != pending.len()).then_some(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autharie_crds::v1alpha::identity_instance::IamRequestPhase;
    use chrono::{TimeZone, Utc};

    fn entry(id: &str) -> IamRequestEntry {
        IamRequestEntry {
            request_id: id.to_string(),
            realm: "master".to_string(),
            operation: r#"{"type":"applications.list","page":1,"limit":25}"#.to_string(),
            sealing_key: r#"{"kty":"RSA","n":"AQAB","e":"AQAB","alg":"RSA-OAEP-256"}"#.to_string(),
            expires_at: Utc.with_ymd_and_hms(2026, 10, 24, 12, 5, 0).unwrap(),
        }
    }

    fn pending(id: &str) -> IamRequestSpec {
        to_spec(&entry(id))
    }

    fn executed(id: &str) -> IamRequestStatus {
        IamRequestStatus {
            request_id: id.to_string(),
            phase: IamRequestPhase::Done,
            message: None,
        }
    }

    #[test]
    fn the_results_secret_is_named_after_the_instance() {
        assert_eq!(
            iam_results_secret_name("deployment-x"),
            "deployment-x-iam-results"
        );
    }

    #[test]
    fn an_entry_is_written_in_the_frozen_crd_shape() {
        let patch = requests_patch(Some("42"), &[pending("a")]);

        assert_eq!(patch["metadata"]["resourceVersion"], json!("42"));
        assert_eq!(
            patch["spec"]["iamRequests"][0],
            json!({
                "requestId": "a",
                "realm": "master",
                "operation": r#"{"type":"applications.list","page":1,"limit":25}"#,
                "sealingKey": r#"{"kty":"RSA","n":"AQAB","e":"AQAB","alg":"RSA-OAEP-256"}"#,
                "expiresAt": "2026-10-24T12:05:00Z",
            })
        );
    }

    #[test]
    fn a_new_request_is_appended_after_the_pending_ones() {
        let requests = with_request(&[pending("a")], &[], &entry("b")).expect("appended");

        let ids: Vec<&str> = requests.iter().map(|r| r.request_id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn a_request_already_pending_is_not_added_again() {
        assert_eq!(with_request(&[pending("a")], &[], &entry("a")), None);
    }

    #[test]
    fn spec_iam_10_a_request_the_operator_already_executed_is_not_added_again() {
        assert_eq!(with_request(&[], &[executed("a")], &entry("a")), None);
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
    fn forgetting_names_each_key_as_null_and_nothing_else() {
        assert_eq!(
            removal_patch(&["a", "b"]),
            json!({ "data": { "a": null, "b": null } })
        );
    }

    #[test]
    fn the_requests_have_a_field_manager_of_their_own() {
        assert_ne!(IAM_FIELD_MANAGER, "genesis");
        assert_ne!(IAM_FIELD_MANAGER, "genesis-webhooks");
    }
}
