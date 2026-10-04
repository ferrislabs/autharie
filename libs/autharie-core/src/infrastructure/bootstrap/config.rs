use std::{path::PathBuf, time::Duration};

use serde_json::{Value, json};

pub const DEFAULT_CHART: &str = "oci://ghcr.io/ferrislabs/charts/autharie-dataplane";

pub const CLOUDNATIVE_PG_VERSION: &str = "0.29.1";
pub const KEDA_VERSION: &str = "2.21.0";
pub const ENVOY_GATEWAY_VERSION: &str = "v1.9.2";

#[derive(Debug, Clone, PartialEq)]
pub struct HelmRelease {
    pub release: String,
    pub chart: String,
    pub repo: Option<String>,
    pub version: String,
    pub namespace: String,
    pub values: Value,
}

impl HelmRelease {
    pub fn new(
        release: impl Into<String>,
        chart: impl Into<String>,
        repo: Option<&str>,
        version: impl Into<String>,
        namespace: impl Into<String>,
    ) -> Self {
        Self {
            release: release.into(),
            chart: chart.into(),
            repo: repo.map(str::to_string),
            version: version.into(),
            namespace: namespace.into(),
            values: json!({}),
        }
    }
}

pub fn default_prerequisites() -> Vec<HelmRelease> {
    vec![
        HelmRelease::new(
            "cnpg",
            "cloudnative-pg",
            Some("https://cloudnative-pg.github.io/charts"),
            CLOUDNATIVE_PG_VERSION,
            "cnpg-system",
        ),
        HelmRelease::new(
            "keda",
            "keda",
            Some("https://kedacore.github.io/charts"),
            KEDA_VERSION,
            "keda",
        ),
        HelmRelease::new(
            "eg",
            "oci://docker.io/envoyproxy/gateway-helm",
            None,
            ENVOY_GATEWAY_VERSION,
            "envoy-gateway-system",
        ),
    ]
}

#[derive(Debug, Clone)]
pub struct HelmConfig {
    pub chart: String,
    pub chart_version: Option<String>,
    pub prerequisites: Vec<HelmRelease>,
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
            prerequisites: default_prerequisites(),
            control_plane_url: control_plane_url.into(),
            herald_issuer: herald_issuer.into(),
            image_registry: None,
            image_tag: None,
            helm_binary: PathBuf::from("helm"),
            timeout: Duration::from_secs(600),
        }
    }
}
