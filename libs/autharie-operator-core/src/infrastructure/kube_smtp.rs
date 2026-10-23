use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, Client, Resource};
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::{IdentityInstance, SmtpStatus};

use crate::domain::OperatorError;
use crate::domain::smtp::SmtpError;
use crate::domain::smtp::ports::{SmtpSecrets, SmtpSecretsPort, SmtpStatusRepository};

use super::ferriskey_admin::name_and_namespace;
use super::kube_webhooks::{is_not_found, removal_patch};

/// The password of each realm, under the realm's name. Genesis writes it.
pub fn smtp_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-smtp")
}

/// The password last given to FerrisKey for each realm, because FerrisKey
/// wants it again with every write and never returns it. The operator's own
/// bookkeeping.
pub fn smtp_applied_secret_name(instance_name: &str) -> String {
    format!("{instance_name}-smtp-applied")
}

pub struct KubeSmtpSecrets {
    client: Client,
}

impl KubeSmtpSecrets {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

fn located(instance: &IdentityInstance) -> Result<(&str, &str), SmtpError> {
    name_and_namespace(instance).ok_or_else(|| SmtpError::Retry {
        message: "the instance has no name or namespace yet".to_string(),
    })
}

/// The message names no key and no value: a Kubernetes error may quote the
/// request, and the request holds a password.
fn kube_error(error: kube::Error) -> SmtpError {
    let reason = match &error {
        kube::Error::Api(response) => format!("status {}", response.code),
        _ => "no answer".to_string(),
    };
    SmtpError::Retry {
        message: format!("the cluster could not read or write an smtp secret ({reason})"),
    }
}

async fn read(secrets: &Api<Secret>, name: &str) -> Result<BTreeMap<String, String>, SmtpError> {
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

impl SmtpSecretsPort for KubeSmtpSecrets {
    async fn load(&self, instance: &IdentityInstance) -> Result<SmtpSecrets, SmtpError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        Ok(SmtpSecrets {
            stored: read(&secrets, &smtp_secret_name(name)).await?,
            applied: read(&secrets, &smtp_applied_secret_name(name)).await?,
        })
    }

    async fn put_applied(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        password: &str,
    ) -> Result<(), SmtpError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let secret_name = smtp_applied_secret_name(name);
        let patch = json!({ "stringData": { realm: password } });

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
                    string_data: Some(BTreeMap::from([(realm.to_string(), password.to_string())])),
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

    async fn forget_applied(
        &self,
        instance: &IdentityInstance,
        realms: &[&str],
    ) -> Result<(), SmtpError> {
        let (name, namespace) = located(instance)?;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let realms: Vec<String> = realms.iter().map(ToString::to_string).collect();

        match secrets
            .patch(
                &smtp_applied_secret_name(name),
                &PatchParams::default(),
                &Patch::Merge(&removal_patch(&realms)),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(kube_error(error)),
        }
    }
}

pub struct KubeSmtpStatus {
    client: Client,
}

impl KubeSmtpStatus {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

/// A merge patch replaces a list whole and leaves a field it does not mention,
/// so the list is always named: an emptied one has to clear what was there.
pub fn smtp_status_patch(statuses: &[SmtpStatus]) -> Value {
    json!({ "status": { "smtp": statuses } })
}

impl SmtpStatusRepository for KubeSmtpStatus {
    async fn patch_smtp_status(
        &self,
        instance: &IdentityInstance,
        statuses: &[SmtpStatus],
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
            &Patch::Merge(&smtp_status_patch(statuses)),
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
    use autharie_crds::v1alpha::identity_instance::SmtpPhase;

    use super::*;

    #[test]
    fn the_secrets_are_named_after_the_instance() {
        assert_eq!(smtp_secret_name("acme"), "acme-smtp");
        assert_eq!(smtp_applied_secret_name("acme"), "acme-smtp-applied");
    }

    #[test]
    fn an_empty_report_clears_the_list() {
        assert_eq!(smtp_status_patch(&[]), json!({ "status": { "smtp": [] } }));
    }

    #[test]
    fn the_patch_touches_nothing_but_the_smtp_field() {
        let patch = smtp_status_patch(&[
            SmtpStatus {
                realm: "acme".to_string(),
                revision: 3,
                phase: SmtpPhase::Applied,
                message: None,
            },
            SmtpStatus {
                realm: "beta".to_string(),
                revision: 1,
                phase: SmtpPhase::Failed,
                message: Some("no password".to_string()),
            },
        ]);

        assert_eq!(patch["status"].as_object().unwrap().len(), 1);
        assert_eq!(
            patch["status"]["smtp"][0],
            json!({ "realm": "acme", "revision": 3, "phase": "Applied" })
        );
        assert_eq!(patch["status"]["smtp"][1]["message"], "no password");
    }

    #[test]
    fn forgetting_names_each_realm_as_null() {
        let patch = removal_patch(&["acme".to_string(), "beta".to_string()]);

        assert_eq!(patch, json!({"data": {"acme": null, "beta": null}}));
    }

    #[test]
    fn a_cluster_error_never_quotes_the_request() {
        let error = kube_error(kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".to_string(),
            message: "invalid value hunter2-secret".to_string(),
            reason: "Invalid".to_string(),
            code: 422,
        }));

        assert!(!error.message().contains("hunter2"));
        assert!(error.message().contains("422"));
        assert!(error.is_retry());
    }
}
