use std::time::Duration;

use k8s_openapi::api::core::v1::ConfigMap;
use kube::Client;
use kube::api::Api;

use crate::domain::error::HeraldError;
use crate::domain::ports::WebhookDeliverySource;

const DELIVERIES_KEY: &str = "deliveries.json";

const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub struct KubeWebhookDeliverySource {
    client: Client,
}

impl KubeWebhookDeliverySource {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn from_env() -> Result<Self, HeraldError> {
        let client = Client::try_default()
            .await
            .map_err(|error| HeraldError::Internal {
                message: format!("failed to build a Kubernetes client: {error}"),
            })?;

        Ok(Self::new(client))
    }
}

impl WebhookDeliverySource for KubeWebhookDeliverySource {
    fn read<'a>(
        &'a self,
        namespace: String,
        config_map: String,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<String>, HeraldError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let api: Api<ConfigMap> = Api::namespaced(self.client.clone(), &namespace);

            let found = tokio::time::timeout(READ_TIMEOUT, api.get_opt(&config_map))
                .await
                .map_err(|_| HeraldError::Internal {
                    message: format!("reading ConfigMap {namespace}/{config_map} timed out"),
                })?
                .map_err(|error| HeraldError::Internal {
                    message: format!(
                        "ConfigMap {namespace}/{config_map} could not be read: {error}"
                    ),
                })?;

            Ok(found.and_then(|config_map| config_map.data?.remove(DELIVERIES_KEY)))
        })
    }
}
