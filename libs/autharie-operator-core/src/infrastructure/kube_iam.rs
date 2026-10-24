use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, Client, Resource};
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::{IamRequestStatus, IdentityInstance};

use crate::domain::OperatorError;
use crate::domain::iam::IamError;
use crate::domain::iam::ports::{IamResultsPort, IamStatusRepository};

use super::ferriskey_admin::name_and_namespace;
use super::kube_webhooks::{is_not_found, removal_patch};

pub fn iam_results_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-iam-results")
}

pub struct KubeIamResults {
    client: Client,
}

impl KubeIamResults {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

fn located(instance: &IdentityInstance) -> Result<(&str, &str), IamError> {
    name_and_namespace(instance).ok_or_else(|| IamError::Retry {
        message: "the instance has no name or namespace yet".to_string(),
    })
}

fn kube_error(error: kube::Error) -> IamError {
    match &error {
        kube::Error::Api(response) if matches!(response.code, 413 | 422) => {
            IamError::rejected("the result does not fit in the cluster")
        }
        kube::Error::Api(response) => IamError::Retry {
            message: format!(
                "the cluster could not read or write an IAM result (status {})",
                response.code
            ),
        },
        _ => IamError::Retry {
            message: "the cluster could not read or write an IAM result (no answer)".to_string(),
        },
    }
}

impl IamResultsPort for KubeIamResults {
    async fn put(
        &self,
        instance: &IdentityInstance,
        request_id: &str,
        envelope: &str,
    ) -> Result<(), IamError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let secret_name = iam_results_secret_name(name);
        let patch = json!({ "stringData": { request_id: envelope } });

        match secrets
            .patch(&secret_name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => {
                let secret = Secret {
                    metadata: ObjectMeta {
                        name: Some(secret_name),
                        namespace: Some(namespace.to_string()),
                        owner_references: instance
                            .controller_owner_ref(&())
                            .map(|owner| vec![owner]),
                        ..Default::default()
                    },
                    string_data: Some(BTreeMap::from([(
                        request_id.to_string(),
                        envelope.to_string(),
                    )])),
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

    async fn forget(
        &self,
        instance: &IdentityInstance,
        request_ids: &[&str],
    ) -> Result<(), IamError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let ids: Vec<String> = request_ids.iter().map(ToString::to_string).collect();

        match secrets
            .patch(
                &iam_results_secret_name(name),
                &PatchParams::default(),
                &Patch::Merge(&removal_patch(&ids)),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(kube_error(error)),
        }
    }
}

pub struct KubeIamStatus {
    client: Client,
}

impl KubeIamStatus {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

pub fn iam_requests_patch(requests: &[IamRequestStatus]) -> Value {
    json!({ "status": { "iam": { "requests": requests } } })
}

fn located_for_status(instance: &IdentityInstance) -> Result<(&str, &str), OperatorError> {
    let name = instance
        .metadata
        .name
        .as_deref()
        .ok_or(OperatorError::MissingName)?;
    let namespace =
        instance
            .metadata
            .namespace
            .as_deref()
            .ok_or_else(|| OperatorError::MissingNamespace {
                name: name.to_string(),
            })?;
    Ok((name, namespace))
}

fn requests_of(instance: IdentityInstance) -> Vec<IamRequestStatus> {
    instance
        .status
        .and_then(|status| status.iam)
        .map(|iam| iam.requests)
        .unwrap_or_default()
}

impl IamStatusRepository for KubeIamStatus {
    async fn recorded(
        &self,
        instance: &IdentityInstance,
    ) -> Result<Vec<IamRequestStatus>, OperatorError> {
        let (name, namespace) = located_for_status(instance)?;
        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), namespace);

        api.get(name)
            .await
            .map(requests_of)
            .map_err(|error| OperatorError::Kube {
                message: error.to_string(),
            })
    }

    async fn patch_requests(
        &self,
        instance: &IdentityInstance,
        requests: &[IamRequestStatus],
    ) -> Result<(), OperatorError> {
        let (name, namespace) = located_for_status(instance)?;
        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), namespace);

        api.patch_status(
            name,
            &PatchParams::default(),
            &Patch::Merge(&iam_requests_patch(requests)),
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
        IamRequestPhase, IamStatus, IdentityInstanceSpec, IdentityInstanceStatus,
    };

    use super::*;

    fn status(id: &str, phase: IamRequestPhase, message: Option<&str>) -> IamRequestStatus {
        IamRequestStatus {
            request_id: id.to_string(),
            phase,
            message: message.map(ToString::to_string),
        }
    }

    #[test]
    fn the_results_secret_is_named_after_the_instance() {
        assert_eq!(iam_results_secret_name("acme"), "acme-iam-results");
    }

    #[test]
    fn the_patch_touches_nothing_but_the_request_list_under_status_iam() {
        let patch = iam_requests_patch(&[
            status("r1", IamRequestPhase::Done, None),
            status("r2", IamRequestPhase::Failed, Some("result too large")),
        ]);

        assert_eq!(
            patch,
            json!({ "status": { "iam": { "requests": [
                { "requestId": "r1", "phase": "Done" },
                { "requestId": "r2", "phase": "Failed", "message": "result too large" }
            ] } } })
        );
    }

    #[test]
    fn an_empty_list_is_named_so_it_clears_what_was_there() {
        assert_eq!(
            iam_requests_patch(&[]),
            json!({ "status": { "iam": { "requests": [] } } })
        );
    }

    #[test]
    fn forgetting_names_each_key_as_null() {
        let patch = removal_patch(&["a".to_string(), "b".to_string()]);

        assert_eq!(patch, json!({ "data": { "a": null, "b": null } }));
    }

    #[test]
    fn the_recorded_requests_are_read_from_status_iam() {
        let mut instance = IdentityInstance::new(
            "acme",
            serde_json::from_value::<IdentityInstanceSpec>(json!({
                "organisationId": "o", "provider": "ferriskey", "version": "1",
                "hostname": "h",
                "database": { "managedCluster": { "storage": { "size": "1Gi" }, "resources": {} } }
            }))
            .unwrap(),
        );
        assert!(requests_of(instance.clone()).is_empty());

        instance.status = Some(IdentityInstanceStatus {
            iam: Some(IamStatus {
                phase: None,
                message: None,
                applied: None,
                observed_at: None,
                requests: vec![status("r1", IamRequestPhase::Done, None)],
            }),
            ..Default::default()
        });

        assert_eq!(requests_of(instance).len(), 1);
    }

    #[test]
    fn a_result_the_cluster_refuses_for_size_fails_the_request_and_other_errors_retry() {
        let api = |code| {
            kube_error(kube::Error::Api(kube::core::ErrorResponse {
                status: "Failure".to_string(),
                message: "Secret is invalid: data: Too long: SEALED-CONTENT".to_string(),
                reason: "Invalid".to_string(),
                code,
            }))
        };

        assert!(!api(422).is_retry());
        assert!(!api(413).is_retry());
        assert!(api(500).is_retry());
        assert!(api(409).is_retry());
    }

    #[test]
    fn a_cluster_error_never_quotes_the_request() {
        let error = kube_error(kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".to_string(),
            message: "invalid value SEALED-CONTENT".to_string(),
            reason: "Invalid".to_string(),
            code: 500,
        }));

        assert!(!error.message().contains("SEALED"));
        assert!(error.message().contains("500"));
    }
}
