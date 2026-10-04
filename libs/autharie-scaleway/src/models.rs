use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub(crate) struct PrivateNetwork {
    pub id: String,
}

#[derive(Deserialize)]
pub(crate) struct Cluster {
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Deserialize)]
pub(crate) struct Pool {
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub node_type: String,
    #[serde(default)]
    pub size: u32,
}

#[derive(Deserialize)]
pub(crate) struct VersionList {
    #[serde(default)]
    pub versions: Vec<Version>,
}

#[derive(Deserialize)]
pub(crate) struct Version {
    pub name: String,
}

#[derive(Deserialize)]
pub(crate) struct ClusterTypeList {
    #[serde(default)]
    pub cluster_types: Vec<ClusterType>,
}

#[derive(Deserialize)]
pub(crate) struct ClusterType {
    pub name: String,
    #[serde(default)]
    pub availability: String,
    #[serde(default)]
    pub dedicated: bool,
}

#[derive(Deserialize)]
pub(crate) struct ServerTypeList {
    #[serde(default)]
    pub servers: HashMap<String, ServerType>,
}

#[derive(Deserialize)]
pub(crate) struct ServerType {
    #[serde(default)]
    pub hourly_price: f64,
    #[serde(default)]
    pub ncpus: u32,
    #[serde(default)]
    pub ram: u64,
}

#[derive(Deserialize)]
pub(crate) struct ProductList {
    #[serde(default)]
    pub products: Vec<Product>,
}

#[derive(Deserialize)]
pub(crate) struct Product {
    #[serde(default)]
    pub variant: String,
    pub price: Option<ProductPrice>,
    pub unit_of_measure: Option<UnitOfMeasure>,
    pub properties: Option<ProductProperties>,
}

#[derive(Deserialize)]
pub(crate) struct ProductPrice {
    pub retail_price: Option<Money>,
}

#[derive(Deserialize)]
pub(crate) struct Money {
    #[serde(default)]
    pub units: i64,
    #[serde(default)]
    pub nanos: i32,
}

#[derive(Deserialize)]
pub(crate) struct UnitOfMeasure {
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize)]
pub(crate) struct ProductProperties {
    pub kubernetes: Option<KubernetesProperties>,
}

#[derive(Deserialize)]
pub(crate) struct KubernetesProperties {
    pub kapsule_control_plane: Option<Value>,
}

#[derive(Deserialize)]
pub(crate) struct ApiKey {
    pub application_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Project {
    pub organization_id: String,
}

#[derive(Deserialize)]
pub(crate) struct PolicyList {
    #[serde(default)]
    pub policies: Vec<Policy>,
}

#[derive(Deserialize)]
pub(crate) struct Policy {
    pub id: String,
}

#[derive(Deserialize)]
pub(crate) struct RuleList {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Deserialize)]
pub(crate) struct Rule {
    #[serde(default)]
    pub permission_set_names: Vec<String>,
}
