use std::{fs, path::PathBuf};

use autharie_crds::v1alpha::{
    identity_dataplane_upgrade::IdentityDataplaneUpgrade,
    identity_instance::IdentityInstance,
    identity_instance_backup::{IdentityInstanceBackup, IdentityInstanceBackupSchedule},
    identity_instance_upgrade::IdentityInstanceUpgrade,
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::CustomResourceExt;

fn render(crds: &[CustomResourceDefinition]) -> String {
    let mut out = String::new();
    for (index, crd) in crds.iter().enumerate() {
        out.push_str(&serde_yaml::to_string(crd).expect("a CRD serialises"));
        if index + 1 < crds.len() {
            out.push_str("---\n");
        }
    }
    out
}

fn generated() -> Vec<(&'static str, String)> {
    vec![
        ("identity-instance.yaml", render(&[IdentityInstance::crd()])),
        (
            "identity-instance-upgrade.yaml",
            render(&[IdentityInstanceUpgrade::crd()]),
        ),
        (
            "identity-instance-backup.yaml",
            render(&[
                IdentityInstanceBackup::crd(),
                IdentityInstanceBackupSchedule::crd(),
            ]),
        ),
        (
            "identity-dataplane-upgrade.yaml",
            render(&[IdentityDataplaneUpgrade::crd()]),
        ),
    ]
}

fn repository_file(relative: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("{relative} could not be read: {error}"))
}

#[test]
fn the_committed_crds_are_what_the_code_generates() {
    for (file, expected) in generated() {
        assert_eq!(
            repository_file(&format!("k8s/crds/{file}")),
            expected,
            "k8s/crds/{file} is stale: run `make crds`"
        );
    }
}

#[test]
fn the_data_plane_chart_carries_the_same_crds() {
    for (file, expected) in generated() {
        assert_eq!(
            repository_file(&format!("charts/autharie-dataplane/crds/{file}")),
            expected,
            "charts/autharie-dataplane/crds/{file} is stale: run `make crds`"
        );
    }
}
