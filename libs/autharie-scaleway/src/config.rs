use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api.scaleway.com";

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ScalewayConfig {
    pub base_url: String,
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    pub poll_interval: Duration,
    pub poll_attempts: u32,
    pub zone_index: u8,
    pub kubernetes_version: Option<String>,
    pub cni: String,
    pub min_node_memory_bytes: u64,
}

impl Default for ScalewayConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_secs(15),
            poll_attempts: 80,
            zone_index: 1,
            kubernetes_version: None,
            cni: "cilium".to_string(),
            min_node_memory_bytes: 4 * GIB,
        }
    }
}
