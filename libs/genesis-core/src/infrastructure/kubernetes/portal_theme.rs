use autharie_crds::v1alpha::identity_instance::{IdentityInstance, PortalThemeSpec};
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use serde_json::{Value, json};
use tracing::info;

use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::portal_theme_payload::PortalThemePayloadV1;
use crate::domain::error::GenesisError;
use crate::domain::ports::PortalThemePort;

pub const PORTAL_THEME_FIELD_MANAGER: &str = "genesis-portal-theme";

pub struct KubePortalThemePort {
    client: Client,
}

impl KubePortalThemePort {
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

impl PortalThemePort for KubePortalThemePort {
    async fn revision<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        realm: &'a str,
    ) -> Result<Option<u64>, GenesisError> {
        let instance = self.instance(reference).await?;

        Ok(instance
            .spec
            .portal_themes
            .iter()
            .find(|entry| entry.realm == realm)
            .map(|entry| entry.revision))
    }

    async fn apply<'a>(
        &'a self,
        reference: &'a IdentityInstanceRef,
        desired: &'a PortalThemePayloadV1,
    ) -> Result<(), GenesisError> {
        let instance = self.instance(reference).await?;

        let Some(entries) = with_entry(&instance.spec.portal_themes, desired) else {
            return Ok(());
        };

        let api: Api<IdentityInstance> = Api::namespaced(self.client.clone(), &reference.namespace);

        info!(
            name = %reference.name,
            namespace = %reference.namespace,
            realm = %desired.realm,
            "setting the portal theme entry"
        );

        api.patch(
            &reference.name,
            &PatchParams {
                field_manager: Some(PORTAL_THEME_FIELD_MANAGER.to_string()),
                ..PatchParams::default()
            },
            &Patch::Merge(&portal_themes_patch(
                instance.metadata.resource_version.as_deref(),
                &entries,
            )),
        )
        .await
        .map_err(|error| kubernetes_error("set the portal theme entry", &error))?;

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

fn to_entry(desired: &PortalThemePayloadV1) -> PortalThemeSpec {
    PortalThemeSpec {
        realm: desired.realm.clone(),
        revision: desired.revision,
        config: Value::Object(desired.config.clone()),
    }
}

pub fn with_entry(
    current: &[PortalThemeSpec],
    desired: &PortalThemePayloadV1,
) -> Option<Vec<PortalThemeSpec>> {
    let entry = to_entry(desired);
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

fn portal_themes_patch(resource_version: Option<&str>, entries: &[PortalThemeSpec]) -> Value {
    json!({
        "metadata": { "resourceVersion": resource_version },
        "spec": { "portalThemes": entries },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const DEPLOYMENT: Uuid = Uuid::from_u128(1);

    fn payload(realm: &str, revision: u64, config: Value) -> PortalThemePayloadV1 {
        PortalThemePayloadV1::from_value(&json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "tenant-a",
            "revision": revision,
            "realm": realm,
            "config": config,
        }))
        .expect("valid")
    }

    fn colour(hex: &str) -> Value {
        json!({ "colors": { "primary": hex } })
    }

    fn entry(realm: &str, revision: u64, hex: &str) -> PortalThemeSpec {
        to_entry(&payload(realm, revision, colour(hex)))
    }

    #[test]
    fn a_new_realm_is_appended() {
        let desired = payload("master", 1, colour("#111111"));

        let entries = with_entry(&[], &desired).expect("written");

        assert_eq!(entries, vec![entry("master", 1, "#111111")]);
    }

    #[test]
    fn a_newer_revision_replaces_the_realms_entry_in_place() {
        let current = [entry("master", 1, "#111111"), entry("acme", 7, "#aaaaaa")];

        let entries =
            with_entry(&current, &payload("master", 2, colour("#222222"))).expect("written");

        assert_eq!(
            entries,
            vec![entry("master", 2, "#222222"), entry("acme", 7, "#aaaaaa")]
        );
    }

    #[test]
    fn an_older_or_equal_revision_changes_nothing() {
        let current = [entry("master", 5, "#111111")];

        assert_eq!(
            with_entry(&current, &payload("master", 4, colour("#222222"))),
            None
        );
        assert_eq!(
            with_entry(&current, &payload("master", 5, colour("#222222"))),
            None
        );
    }

    #[test]
    fn the_other_realms_are_never_touched() {
        let current = [
            entry("acme", 9, "#aaaaaa"),
            entry("zeta", 4, "#bbbbbb"),
            entry("master", 1, "#111111"),
        ];

        let entries =
            with_entry(&current, &payload("master", 2, colour("#222222"))).expect("written");

        assert_eq!(entries[..2], current[..2]);
        assert_eq!(entries[2], entry("master", 2, "#222222"));
    }

    #[test]
    fn an_empty_config_is_written_as_an_empty_object() {
        let entries = with_entry(&[], &payload("master", 3, json!({}))).expect("written");

        assert_eq!(
            serde_json::to_value(&entries[0]).expect("serialises"),
            json!({ "realm": "master", "revision": 3, "config": {} })
        );
    }

    #[test]
    fn the_entry_carries_the_frozen_spec_fields_with_the_config_untouched() {
        let config = json!({
            "colors": { "primary": "#112233" },
            "fonts": { "baseSize": 16 },
        });

        let entries = with_entry(&[], &payload("master", 4, config.clone())).expect("written");

        assert_eq!(
            serde_json::to_value(&entries[0]).expect("serialises"),
            json!({ "realm": "master", "revision": 4, "config": config })
        );
    }

    #[test]
    fn the_spec_patch_writes_the_whole_list_and_pins_the_resource_version() {
        let entries = [entry("acme", 1, "#aaaaaa"), entry("master", 2, "#111111")];

        let patch = portal_themes_patch(Some("42"), &entries);

        assert_eq!(patch["metadata"]["resourceVersion"], json!("42"));
        assert_eq!(
            patch["spec"]["portalThemes"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(patch["spec"]["portalThemes"][0]["realm"], json!("acme"));
        assert_eq!(patch["spec"]["portalThemes"][1]["revision"], json!(2));
        assert!(
            patch["spec"]
                .as_object()
                .is_some_and(|spec| spec.len() == 1)
        );
    }

    #[test]
    fn the_portal_theme_writes_have_a_field_manager_of_their_own() {
        assert_ne!(PORTAL_THEME_FIELD_MANAGER, "genesis");
        assert_ne!(PORTAL_THEME_FIELD_MANAGER, "genesis-smtp");
        assert_ne!(PORTAL_THEME_FIELD_MANAGER, "genesis-webhooks");
    }
}
