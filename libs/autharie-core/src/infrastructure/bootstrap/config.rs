use std::{path::PathBuf, time::Duration};

pub const DEFAULT_CHART: &str = "oci://ghcr.io/ferrislabs/charts/autharie-dataplane";

#[derive(Debug, Clone)]
pub struct HelmConfig {
    pub chart: String,
    pub chart_version: Option<String>,
    pub control_plane_url: String,
    pub herald_issuer: String,
    pub image_registry: Option<String>,
    pub image_tag: Option<String>,
    pub helm_binary: PathBuf,
    pub timeout: Duration,
}

impl HelmConfig {
    pub fn new(control_plane_url: impl Into<String>, herald_issuer: impl Into<String>) -> Self {
        Self {
            chart: DEFAULT_CHART.to_string(),
            chart_version: None,
            control_plane_url: control_plane_url.into(),
            herald_issuer: herald_issuer.into(),
            image_registry: None,
            image_tag: None,
            helm_binary: PathBuf::from("helm"),
            timeout: Duration::from_secs(600),
        }
    }
}
