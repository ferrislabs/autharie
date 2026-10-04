use autharie_domain::dataplane::cluster_profile::ClusterProfile;
use httpmock::{
    Method::{DELETE, GET, PATCH, POST},
    Mock, MockServer,
};
use serde_json::{Value, json};

const GIB: u64 = 1024 * 1024 * 1024;
const BASE: &str = "/k8s/v1/regions/fr-par";
const NETWORKS: &str = "/vpc/v2/regions/fr-par/private-networks";

pub const PRO2_S_CPUS: u32 = 4;
pub const PRO2_S_MEMORY_MIB: u32 = 16 * 1024;

pub fn kubeconfig(token: &str) -> String {
    format!("apiVersion: v1\ntoken: {token}\n")
}

fn not_found() -> Value {
    json!({"type": "not_found", "message": "gone"})
}

async fn servers<'a>(server: &'a MockServer, secret_key: &str) -> Mock<'a> {
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/instance/v1/zones/fr-par-1/products/servers")
                .header("x-auth-token", secret_key);
            then.status(200).json_body(json!({
                "servers": {
                    "PRO2-S": {"hourly_price": 0.1, "ncpus": PRO2_S_CPUS, "ram": 16 * GIB},
                    "PRO2-M": {"hourly_price": 0.2, "ncpus": 8, "ram": 32 * GIB}
                }
            }));
        })
        .await
}

async fn network<'a>(server: &'a MockServer) -> Mock<'a> {
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path(NETWORKS)
                .json_body_partial(r#"{"project_id":"proj-1"}"#);
            then.status(200).json_body(json!({"id": "pn-1"}));
        })
        .await
}

async fn versions<'a>(server: &'a MockServer) -> Mock<'a> {
    server
        .mock_async(|when, then| {
            when.method(GET).path(format!("{BASE}/versions"));
            then.status(200)
                .json_body(json!({"versions": [{"name": "1.30.2"}, {"name": "1.31.4"}]}));
        })
        .await
}

pub struct Creation<'a> {
    network: Mock<'a>,
    cluster: Mock<'a>,
    pool: Mock<'a>,
    others: Vec<Mock<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreationHits {
    pub networks: usize,
    pub clusters: usize,
    pub pools: usize,
}

impl Creation<'_> {
    pub async fn hits(&self) -> CreationHits {
        CreationHits {
            networks: self.network.hits_async().await,
            clusters: self.cluster.hits_async().await,
            pools: self.pool.hits_async().await,
        }
    }

    pub async fn remove(self) {
        self.network.delete_async().await;
        self.cluster.delete_async().await;
        self.pool.delete_async().await;
        for mock in &self.others {
            mock.delete_async().await;
        }
    }
}

pub async fn creation<'a>(
    server: &'a MockServer,
    secret_key: &str,
    profile: &ClusterProfile,
    kubeconfig_body: &str,
) -> Creation<'a> {
    let control_plane = profile.control_plane().id.as_str().to_string();
    let autoscaling = profile.mode().limits().autoscaling;
    let mut pool_body = json!({
        "node_type": profile.node_type().as_str(),
        "size": profile.min_nodes(),
        "autoscaling": autoscaling,
        "zone": "fr-par-1",
    });
    if autoscaling {
        pool_body["min_size"] = json!(profile.min_nodes());
        pool_body["max_size"] = json!(profile.max_nodes());
    }

    let mut others = vec![servers(server, secret_key).await, versions(server).await];
    let network = network(server).await;
    let cluster = server
        .mock_async(|when, then| {
            when.method(POST)
                .path(format!("{BASE}/clusters"))
                .header("x-auth-token", secret_key)
                .json_body_partial(
                    json!({
                        "version": "1.31.4",
                        "type": control_plane,
                        "private_network_id": "pn-1",
                        "project_id": "proj-1",
                    })
                    .to_string(),
                );
            then.status(200)
                .json_body(json!({"id": "cl-1", "status": "creating", "type": control_plane}));
        })
        .await;
    let pool = server
        .mock_async(|when, then| {
            when.method(POST)
                .path(format!("{BASE}/clusters/cl-1/pools"))
                .json_body_partial(pool_body.to_string());
            then.status(200)
                .json_body(json!({"id": "pool-1", "status": "scaling"}));
        })
        .await;
    others.push(
        server
            .mock_async(|when, then| {
                when.method(GET).path(format!("{BASE}/clusters/cl-1"));
                then.status(200)
                    .json_body(json!({"id": "cl-1", "status": "ready", "type": control_plane}));
            })
            .await,
    );
    others.push(
        server
            .mock_async(|when, then| {
                when.method(GET).path(format!("{BASE}/pools/pool-1"));
                then.status(200).json_body(json!({
                    "id": "pool-1",
                    "status": "ready",
                    "node_type": profile.node_type().as_str(),
                    "size": profile.min_nodes(),
                }));
            })
            .await,
    );
    others.push(
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(format!("{BASE}/clusters/cl-1/kubeconfig"))
                    .query_param("dl", "1");
                then.status(200).body(kubeconfig_body);
            })
            .await,
    );

    Creation {
        network,
        cluster,
        pool,
        others,
    }
}

pub struct Refusal<'a> {
    cluster_attempts: Mock<'a>,
    network_deletions: Mock<'a>,
    others: Vec<Mock<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalHits {
    pub cluster_attempts: usize,
    pub network_deletions: usize,
}

impl Refusal<'_> {
    pub async fn hits(&self) -> RefusalHits {
        RefusalHits {
            cluster_attempts: self.cluster_attempts.hits_async().await,
            network_deletions: self.network_deletions.hits_async().await,
        }
    }

    pub async fn remove(self) {
        self.cluster_attempts.delete_async().await;
        self.network_deletions.delete_async().await;
        for mock in &self.others {
            mock.delete_async().await;
        }
    }
}

pub async fn quota_refusal<'a>(server: &'a MockServer, secret_key: &str) -> Refusal<'a> {
    let others = vec![
        servers(server, secret_key).await,
        network(server).await,
        versions(server).await,
    ];
    let cluster_attempts = server
        .mock_async(|when, then| {
            when.method(POST).path(format!("{BASE}/clusters"));
            then.status(403).json_body(
                json!({"type": "quotas_exceeded", "message": "quota for clusters reached"}),
            );
        })
        .await;
    let network_deletions = server
        .mock_async(|when, then| {
            when.method(DELETE).path(format!("{NETWORKS}/pn-1"));
            then.status(204);
        })
        .await;
    Refusal {
        cluster_attempts,
        network_deletions,
        others,
    }
}

pub struct Teardown<'a> {
    pool: Mock<'a>,
    cluster: Mock<'a>,
    network: Mock<'a>,
    gone: Mock<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeardownHits {
    pub pools: usize,
    pub clusters: usize,
    pub networks: usize,
}

impl Teardown<'_> {
    pub async fn hits(&self) -> TeardownHits {
        TeardownHits {
            pools: self.pool.hits_async().await,
            clusters: self.cluster.hits_async().await,
            networks: self.network.hits_async().await,
        }
    }

    pub async fn remove(self) {
        self.pool.delete_async().await;
        self.cluster.delete_async().await;
        self.network.delete_async().await;
        self.gone.delete_async().await;
    }
}

pub async fn teardown(server: &MockServer) -> Teardown<'_> {
    let pool = server
        .mock_async(|when, then| {
            when.method(DELETE).path(format!("{BASE}/pools/pool-1"));
            then.status(200)
                .json_body(json!({"id": "pool-1", "status": "deleting"}));
        })
        .await;
    let cluster = server
        .mock_async(|when, then| {
            when.method(DELETE)
                .path(format!("{BASE}/clusters/cl-1"))
                .query_param("with_additional_resources", "true");
            then.status(200)
                .json_body(json!({"id": "cl-1", "status": "deleting"}));
        })
        .await;
    let network = server
        .mock_async(|when, then| {
            when.method(DELETE).path(format!("{NETWORKS}/pn-1"));
            then.status(204);
        })
        .await;
    let gone = server
        .mock_async(|when, then| {
            when.method(GET);
            then.status(404).json_body(not_found());
        })
        .await;
    Teardown {
        pool,
        cluster,
        network,
        gone,
    }
}

pub struct Resizing<'a> {
    patch: Mock<'a>,
    others: Vec<Mock<'a>>,
}

impl Resizing<'_> {
    pub async fn patches(&self) -> usize {
        self.patch.hits_async().await
    }

    pub async fn remove(self) {
        self.patch.delete_async().await;
        for mock in &self.others {
            mock.delete_async().await;
        }
    }
}

pub async fn resizing<'a>(
    server: &'a MockServer,
    node_type: &str,
    cluster_type: &str,
    current_size: u32,
    expected_patch: &Value,
) -> Resizing<'a> {
    let others = vec![
        server
            .mock_async(|when, then| {
                when.method(GET).path(format!("{BASE}/pools/pool-1"));
                then.status(200).json_body(json!({
                    "id": "pool-1",
                    "status": "ready",
                    "node_type": node_type,
                    "size": current_size,
                }));
            })
            .await,
        server
            .mock_async(|when, then| {
                when.method(GET).path(format!("{BASE}/clusters/cl-1"));
                then.status(200)
                    .json_body(json!({"id": "cl-1", "status": "ready", "type": cluster_type}));
            })
            .await,
    ];
    let patch = server
        .mock_async(|when, then| {
            when.method(PATCH)
                .path(format!("{BASE}/pools/pool-1"))
                .json_body_partial(expected_patch.to_string());
            then.status(200)
                .json_body(json!({"id": "pool-1", "status": "scaling"}));
        })
        .await;
    Resizing { patch, others }
}
