use crate::config::ScalewayConfig;

pub(crate) struct Layout {
    pub region: String,
    pub zone: String,
}

impl Layout {
    pub(crate) fn new(config: &ScalewayConfig, region: &str) -> Self {
        Self {
            region: region.to_string(),
            zone: format!("{region}-{}", config.zone_index),
        }
    }

    pub(crate) fn clusters(&self) -> String {
        format!("/k8s/v1/regions/{}/clusters", self.region)
    }

    pub(crate) fn cluster(&self, id: &str) -> String {
        format!("/k8s/v1/regions/{}/clusters/{id}", self.region)
    }

    pub(crate) fn pools(&self, cluster_id: &str) -> String {
        format!(
            "/k8s/v1/regions/{}/clusters/{cluster_id}/pools",
            self.region
        )
    }

    pub(crate) fn pool(&self, id: &str) -> String {
        format!("/k8s/v1/regions/{}/pools/{id}", self.region)
    }

    pub(crate) fn private_networks(&self) -> String {
        format!("/vpc/v2/regions/{}/private-networks", self.region)
    }

    pub(crate) fn private_network(&self, id: &str) -> String {
        format!("/vpc/v2/regions/{}/private-networks/{id}", self.region)
    }

    pub(crate) fn versions(&self) -> String {
        format!("/k8s/v1/regions/{}/versions", self.region)
    }

    pub(crate) fn cluster_types(&self) -> String {
        format!("/k8s/v1/regions/{}/cluster-types", self.region)
    }

    pub(crate) fn server_types(&self) -> String {
        format!("/instance/v1/zones/{}/products/servers", self.zone)
    }
}
