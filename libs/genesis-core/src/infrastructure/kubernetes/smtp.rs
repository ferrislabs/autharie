use std::collections::BTreeMap;

use autharie_crds::v1alpha::identity_instance::{
    IdentityInstance, SmtpEncryption as CrdEncryption, SmtpSpec,
};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{Patch, PatchParams, PostParams};
use kube::core::ObjectMeta;
use kube::{Api, Client, Resource};
use serde_json::{Value, json};
use tracing::info;

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::smtp_payload::{
    PasswordChange, SmtpEncryption, SmtpPayloadV1, SmtpSettings,
};
use crate::domain::error::GenesisError;
use crate::domain::ports::SmtpPort;

pub const SMTP_FIELD_MANAGER: &str = "genesis-smtp";

pub struct KubeSmtpPort {
    client: Client,
}

impl KubeSmtpPort {
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

impl SmtpPort for KubeSmtpPort {
    async fn revision<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        realm: &'a str,
    ) -> Result<Option<u64>, GenesisError> {
        let instance = self.instance(reference).await?;

        Ok(instance
            .spec
            .smtp
            .iter()
            .find(|entry| entry.realm == realm)
            .map(|entry| entry.revision))
    }

    async fn write_password<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        realm: &'a str,
        change: PasswordChange<'a>,
    ) -> Result<(), GenesisError> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &reference.namespace);
        let name = secret_name(reference);

        let existing = secrets
            .get_opt(&name)
            .await
            .map_err(|error| kubernetes_error("read the SMTP Secret", &error))?;

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            realm,
            "writing the SMTP password Secret"
        );

        match (existing, change) {
            (None, PasswordChange::Remove) => Ok(()),
            (None, PasswordChange::Set(password)) => {
                let instance = self.instance(reference).await?;
                let owner =
                    instance
                        .controller_owner_ref(&())
                        .ok_or_else(|| GenesisError::Kubernetes {
                            message: format!("IdentityInstance {} has no uid", reference.name),
                        })?;

                secrets
                    .create(
                        &PostParams {
                            field_manager: Some(SMTP_FIELD_MANAGER.to_string()),
                            ..PostParams::default()
                        },
                        &password_secret(reference, realm, password, owner),
                    )
                    .await
                    .map_err(|error| kubernetes_error("create the SMTP Secret", &error))?;
                Ok(())
            }
            (Some(_), change) => {
                secrets
                    .patch(
                        &name,
                        &PatchParams {
                            field_manager: Some(SMTP_FIELD_MANAGER.to_string()),
                            ..PatchParams::default()
                        },
                        &Patch::Merge(&password_patch(realm, change)),
                    )
                    .await
                    .map_err(|error| kubernetes_error("write the SMTP Secret", &error))?;
                Ok(())
            }
        }
    }

    async fn apply<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        desired: &'a SmtpPayloadV1,
    ) -> Result<(), GenesisError> {
        let instance = self.instance(reference).await?;

        let Some(entries) = with_entry(&instance.spec.smtp, desired) else {
            return Ok(());
        };

        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), &reference.namespace);

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            realm = %desired.realm,
            "setting the smtp entry"
        );

        api.patch(
            &reference.name,
            &PatchParams {
                field_manager: Some(SMTP_FIELD_MANAGER.to_string()),
                ..PatchParams::default()
            },
            &Patch::Merge(&smtp_patch(
                instance.metadata.resource_version.as_deref(),
                &entries,
            )),
        )
        .await
        .map_err(|error| kubernetes_error("set the smtp entry", &error))?;

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

pub fn secret_name(reference: &IdentityInstanceRef) -> String {
    format!("{}-smtp", reference.name)
}

fn password_secret(
    reference: &IdentityInstanceRef,
    realm: &str,
    password: &str,
    owner: OwnerReference,
) -> Secret {
    let data: BTreeMap<String, ByteString> =
        BTreeMap::from([(realm.to_string(), ByteString(password.as_bytes().to_vec()))]);

    Secret {
        metadata: ObjectMeta {
            name: Some(secret_name(reference)),
            namespace: Some(reference.namespace.clone()),
            owner_references: Some(vec![owner]),
            ..Default::default()
        },
        type_: Some("Opaque".to_string()),
        data: Some(data),
        ..Default::default()
    }
}

fn password_patch(realm: &str, change: PasswordChange<'_>) -> Value {
    let value = match change {
        PasswordChange::Set(password) => json!(ByteString(password.as_bytes().to_vec())),
        PasswordChange::Remove => Value::Null,
    };

    json!({ "data": { realm: value } })
}

fn to_encryption(encryption: SmtpEncryption) -> CrdEncryption {
    match encryption {
        SmtpEncryption::Tls => CrdEncryption::Tls,
        SmtpEncryption::Starttls => CrdEncryption::Starttls,
        SmtpEncryption::None => CrdEncryption::None,
    }
}

fn to_entry(realm: &str, revision: u64, settings: Option<&SmtpSettings>) -> SmtpSpec {
    match settings {
        Some(settings) => SmtpSpec {
            realm: realm.to_string(),
            revision,
            remove: false,
            host: Some(settings.host.clone()),
            port: Some(settings.port),
            encryption: Some(to_encryption(settings.encryption)),
            username: Some(settings.username.clone()),
            from_email: Some(settings.from_email.clone()),
            from_name: Some(settings.from_name.clone()),
        },
        None => SmtpSpec {
            realm: realm.to_string(),
            revision,
            remove: true,
            host: None,
            port: None,
            encryption: None,
            username: None,
            from_email: None,
            from_name: None,
        },
    }
}

pub fn with_entry(current: &[SmtpSpec], desired: &SmtpPayloadV1) -> Option<Vec<SmtpSpec>> {
    let entry = to_entry(&desired.realm, desired.revision, desired.smtp.as_ref());
    let mut entries = current.to_vec();

    match entries
        .iter_mut()
        .find(|known| known.realm == desired.realm)
    {
        Some(known) if desired.revision <= known.revision => return None,
        Some(known) => *known = entry,
        None => entries.push(entry),
    }

    Some(entries)
}

fn smtp_patch(resource_version: Option<&str>, entries: &[SmtpSpec]) -> Value {
    json!({
        "metadata": { "resourceVersion": resource_version },
        "spec": { "smtp": entries },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);
    const PASSWORD: &str = "hunter2-do-not-print";

    fn reference() -> IdentityInstanceRef {
        IdentityInstanceRef::for_deployment(DEPLOYMENT, "tenant-a")
    }

    fn payload(realm: &str, revision: u64, smtp: Value) -> SmtpPayloadV1 {
        SmtpPayloadV1::from_value(&json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": revision,
            "realm": realm,
            "smtp": smtp,
        }))
        .expect("valid")
    }

    fn settings(host: &str) -> Value {
        json!({
            "host": host,
            "port": 465,
            "encryption": "tls",
            "username": "mailer",
            "from_email": "no-reply@example.com",
            "from_name": "Example",
            "password": PASSWORD,
        })
    }

    fn save(realm: &str, revision: u64, host: &str) -> SmtpPayloadV1 {
        payload(realm, revision, settings(host))
    }

    fn removal(realm: &str, revision: u64) -> SmtpPayloadV1 {
        payload(realm, revision, Value::Null)
    }

    fn entry(realm: &str, revision: u64, host: &str) -> SmtpSpec {
        to_entry(realm, revision, save(realm, revision, host).smtp.as_ref())
    }

    fn tombstone(realm: &str, revision: u64) -> SmtpSpec {
        to_entry(realm, revision, None)
    }

    #[test]
    fn a_new_realm_is_appended() {
        let entries = with_entry(&[], &save("master", 1, "a")).expect("written");

        assert_eq!(entries, vec![entry("master", 1, "a")]);
    }

    #[test]
    fn a_newer_revision_replaces_the_realms_entry_in_place() {
        let current = [entry("master", 1, "a"), entry("acme", 7, "x")];

        let entries = with_entry(&current, &save("master", 2, "b")).expect("written");

        assert_eq!(
            entries,
            vec![entry("master", 2, "b"), entry("acme", 7, "x")]
        );
    }

    #[test]
    fn an_older_or_equal_revision_changes_nothing() {
        let current = [entry("master", 5, "a")];

        assert_eq!(with_entry(&current, &save("master", 4, "b")), None);
        assert_eq!(with_entry(&current, &save("master", 5, "b")), None);
        assert_eq!(with_entry(&current, &removal("master", 5)), None);
    }

    #[test]
    fn a_removal_leaves_a_tombstone_with_nothing_else_in_it() {
        let current = [entry("master", 1, "a")];

        let entries = with_entry(&current, &removal("master", 2)).expect("written");

        assert_eq!(entries, vec![tombstone("master", 2)]);
        assert_eq!(
            serde_json::to_value(&entries[0]).expect("serialises"),
            json!({ "realm": "master", "revision": 2, "remove": true })
        );
    }

    #[test]
    fn a_removal_of_a_realm_never_saved_still_records_its_revision() {
        let entries = with_entry(&[], &removal("master", 3)).expect("written");

        assert_eq!(entries, vec![tombstone("master", 3)]);
    }

    #[test]
    fn an_older_save_does_not_resurrect_a_tombstone() {
        let current = [tombstone("master", 6)];

        assert_eq!(with_entry(&current, &save("master", 5, "a")), None);
        assert_eq!(with_entry(&current, &save("master", 6, "a")), None);
    }

    #[test]
    fn a_newer_save_replaces_a_tombstone() {
        let current = [tombstone("master", 6)];

        let entries = with_entry(&current, &save("master", 7, "a")).expect("written");

        assert_eq!(entries, vec![entry("master", 7, "a")]);
        assert!(!entries[0].remove);
    }

    #[test]
    fn the_other_realms_are_never_touched() {
        let current = [
            entry("acme", 9, "x"),
            tombstone("gone", 4),
            entry("master", 1, "a"),
        ];

        let saved = with_entry(&current, &save("master", 2, "b")).expect("written");
        let removed = with_entry(&current, &removal("master", 2)).expect("written");

        assert_eq!(saved[..2], current[..2]);
        assert_eq!(removed[..2], current[..2]);
    }

    #[test]
    fn the_entry_carries_the_frozen_spec_fields_in_camel_case_and_no_password() {
        let entries = with_entry(&[], &save("master", 4, "smtp.example.com")).expect("written");

        let value = serde_json::to_value(&entries[0]).expect("serialises");

        assert_eq!(
            value,
            json!({
                "realm": "master",
                "revision": 4,
                "host": "smtp.example.com",
                "port": 465,
                "encryption": "tls",
                "username": "mailer",
                "fromEmail": "no-reply@example.com",
                "fromName": "Example",
            })
        );
        assert!(!smtp_patch(None, &entries).to_string().contains(PASSWORD));
    }

    #[test]
    fn the_spec_patch_writes_the_whole_list_and_pins_the_resource_version() {
        let entries = [entry("acme", 1, "x"), tombstone("master", 2)];

        let patch = smtp_patch(Some("42"), &entries);

        assert_eq!(patch["metadata"]["resourceVersion"], json!("42"));
        assert_eq!(patch["spec"]["smtp"].as_array().map(Vec::len), Some(2));
        assert_eq!(patch["spec"]["smtp"][0]["realm"], json!("acme"));
        assert_eq!(
            patch["spec"]["smtp"][1],
            json!({ "realm": "master", "revision": 2, "remove": true })
        );
        assert!(
            patch["spec"]
                .as_object()
                .is_some_and(|spec| spec.len() == 1)
        );
    }

    #[test]
    fn the_secret_is_named_after_the_instance_and_owned_by_it() {
        let owner = OwnerReference {
            api_version: "autharie.fr/v1alpha".to_string(),
            kind: "IdentityInstance".to_string(),
            name: reference().name,
            uid: "uid-1".to_string(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        };

        let secret = password_secret(&reference(), "master", PASSWORD, owner);

        assert_eq!(
            secret.metadata.name.as_deref(),
            Some(format!("deployment-{DEPLOYMENT}-smtp").as_str())
        );
        assert_eq!(secret.metadata.namespace.as_deref(), Some("tenant-a"));
        assert_eq!(
            secret.metadata.owner_references.as_ref().expect("owned")[0].uid,
            "uid-1"
        );
        let data = secret.data.expect("data");
        assert_eq!(data.len(), 1);
        assert_eq!(data["master"].0, PASSWORD.as_bytes().to_vec());
    }

    #[test]
    fn setting_a_password_patches_only_the_realms_key() {
        let patch = password_patch("master", PasswordChange::Set(PASSWORD));

        let data = patch["data"].as_object().expect("data");
        assert_eq!(data.len(), 1);
        let encoded = data["master"].as_str().expect("base64");
        assert_eq!(encoded, "aHVudGVyMi1kby1ub3QtcHJpbnQ=");
        assert!(patch.get("metadata").is_none());
    }

    #[test]
    fn removing_a_password_nulls_only_the_realms_key() {
        let patch = password_patch("acme", PasswordChange::Remove);

        assert_eq!(patch, json!({ "data": { "acme": null } }));
    }

    #[test]
    fn the_smtp_writes_have_a_field_manager_of_their_own() {
        assert_ne!(SMTP_FIELD_MANAGER, "genesis");
        assert_ne!(SMTP_FIELD_MANAGER, "genesis-webhooks");
    }
}
