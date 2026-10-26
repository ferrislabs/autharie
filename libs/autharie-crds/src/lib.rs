pub mod common;
pub mod v1alpha;
pub mod yaml;

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::CustomResourceExt;

use crate::v1alpha::{
    identity_dataplane_upgrade::IdentityDataplaneUpgrade,
    identity_instance::IdentityInstance,
    identity_instance_backup::{IdentityInstanceBackup, IdentityInstanceBackupSchedule},
    identity_instance_upgrade::IdentityInstanceUpgrade,
};

pub fn all_crds() -> Vec<CustomResourceDefinition> {
    vec![
        IdentityInstance::crd(),
        IdentityInstanceUpgrade::crd(),
        IdentityInstanceBackup::crd(),
        IdentityInstanceBackupSchedule::crd(),
        IdentityDataplaneUpgrade::crd(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_crd_the_chart_ships_is_listed() {
        let names: Vec<String> = all_crds()
            .into_iter()
            .map(|crd| crd.metadata.name.expect("a CRD is named"))
            .collect();

        assert_eq!(
            names,
            vec![
                "identityinstances.autharie.fr",
                "identityinstanceupgrades.autharie.fr",
                "identityinstancebackups.autharie.fr",
                "identityinstancebackupschedules.autharie.fr",
                "identitydataplaneupgrades.autharie.fr",
            ]
        );
    }
}
