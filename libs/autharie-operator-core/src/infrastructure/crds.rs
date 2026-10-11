use std::future::Future;

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use tracing::{error, info};

const FIELD_MANAGER: &str = "autharie-operator";

pub async fn apply_crds(client: &Client) -> Vec<String> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let params = PatchParams::apply(FIELD_MANAGER).force();

    apply_each(autharie_crds::all_crds(), |name, crd| {
        let api = api.clone();
        let params = params.clone();
        async move {
            api.patch(&name, &params, &Patch::Apply(&crd))
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
    })
    .await
}

async fn apply_each<F, Fut>(crds: Vec<CustomResourceDefinition>, mut apply: F) -> Vec<String>
where
    F: FnMut(String, CustomResourceDefinition) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut failed = Vec::new();

    for crd in crds {
        let name = crd.metadata.name.clone().unwrap_or_default();
        match apply(name.clone(), crd).await {
            Ok(()) => info!(crd = %name, "applied the CRD"),
            Err(reason) => {
                error!(crd = %name, %reason, "could not apply the CRD, carrying on with the one installed");
                failed.push(name);
            }
        }
    }

    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_apply_is_reported_and_the_rest_still_run() {
        let mut seen = Vec::new();

        let failed = apply_each(autharie_crds::all_crds(), |name, _| {
            seen.push(name.clone());
            let outcome = if name == "identityinstanceupgrades.autharie.fr" {
                Err("forbidden".to_string())
            } else {
                Ok(())
            };
            async move { outcome }
        })
        .await;

        assert_eq!(failed, vec!["identityinstanceupgrades.autharie.fr"]);
        assert_eq!(seen.len(), autharie_crds::all_crds().len());
    }

    #[test]
    fn the_rbac_template_names_every_crd_that_is_applied() {
        let template = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../charts/autharie-dataplane/templates/rbac.yaml"),
        )
        .expect("the RBAC template is readable");

        for crd in autharie_crds::all_crds() {
            let name = crd.metadata.name.expect("a CRD is named");
            assert!(
                template.contains(&format!("\"{name}\"")),
                "{name} is applied at start-up but absent from the operator's resourceNames"
            );
        }
    }
}
