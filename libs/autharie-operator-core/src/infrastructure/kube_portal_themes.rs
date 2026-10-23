use kube::Api;
use kube::Client;
use kube::api::{Patch, PatchParams};
use serde_json::{Value, json};

use autharie_crds::v1alpha::identity_instance::{IdentityInstance, PortalThemeStatus};

use crate::domain::OperatorError;
use crate::domain::portal_themes::ports::PortalThemeStatusRepository;

pub struct KubePortalThemeStatus {
    client: Client,
}

impl KubePortalThemeStatus {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

pub fn portal_theme_status_patch(statuses: &[PortalThemeStatus]) -> Value {
    json!({ "status": { "portalThemes": statuses } })
}

impl PortalThemeStatusRepository for KubePortalThemeStatus {
    async fn patch_portal_theme_status(
        &self,
        instance: &IdentityInstance,
        statuses: &[PortalThemeStatus],
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
            &Patch::Merge(&portal_theme_status_patch(statuses)),
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
    use autharie_crds::v1alpha::identity_instance::PortalThemePhase;

    use super::*;

    #[test]
    fn an_empty_report_clears_the_list() {
        assert_eq!(
            portal_theme_status_patch(&[]),
            json!({ "status": { "portalThemes": [] } })
        );
    }

    #[test]
    fn the_patch_touches_nothing_but_the_portal_themes_field() {
        let patch = portal_theme_status_patch(&[
            PortalThemeStatus {
                realm: "acme".to_string(),
                revision: 3,
                phase: PortalThemePhase::Applied,
                message: None,
            },
            PortalThemeStatus {
                realm: "beta".to_string(),
                revision: 1,
                phase: PortalThemePhase::Failed,
                message: Some("FerrisKey answered 400".to_string()),
            },
        ]);

        assert_eq!(patch["status"].as_object().unwrap().len(), 1);
        assert_eq!(
            patch["status"]["portalThemes"][0],
            json!({ "realm": "acme", "revision": 3, "phase": "Applied" })
        );
        assert_eq!(
            patch["status"]["portalThemes"][1]["message"],
            "FerrisKey answered 400"
        );
        assert_eq!(patch["status"]["portalThemes"][1]["phase"], "Failed");
    }
}
