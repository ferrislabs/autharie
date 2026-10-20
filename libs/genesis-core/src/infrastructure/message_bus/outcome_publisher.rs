use autharie_iggy::iggy::prelude::{IggyClient, IggyMessage, MessageClient, Partitioning};
use autharie_iggy::{OUTCOMES_TOPIC, PARTITION, STREAM, bounded, named};
use serde_json::to_vec;
use tracing::info;

use crate::domain::entities::outcome::DeploymentOutcomeReport;
use crate::domain::error::GenesisError;
use crate::domain::ports::{BoxFuture, OutcomePublisher};

/// Publishes outcomes to the outcomes topic, where every Herald reads them.
///
/// A topic of its own rather than the one actions travel on: an outcome that
/// reached Genesis would fail to parse as an action and be retried for ever.
pub struct IggyOutcomePublisher {
    // One client for the life of the process: it reconnects and signs in
    // again on its own when the broker restarts or drops the connection.
    client: IggyClient,
}

impl IggyOutcomePublisher {
    pub async fn connect(url: &str) -> Result<Self, GenesisError> {
        let client = autharie_iggy::connect_with_retry(url, autharie_iggy::DEFAULT_BUDGET)
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to reach the broker: {e}"),
            })?;

        // Declared here as well as by Herald: whichever starts first must
        // find the topic.
        autharie_iggy::ensure_topic(&client, OUTCOMES_TOPIC)
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to declare the '{OUTCOMES_TOPIC}' topic: {e}"),
            })?;

        Ok(Self { client })
    }
}

impl OutcomePublisher for IggyOutcomePublisher {
    fn publish<'a>(
        &'a self,
        report: DeploymentOutcomeReport,
    ) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = to_vec(&report).map_err(|e| GenesisError::MessageBus {
                message: format!("failed to serialise outcome: {e}"),
            })?;

            info!(
                deployment_id = %report.deployment_id,
                outcome = %report.outcome,
                "reporting outcome"
            );

            let stream = named(STREAM).map_err(|e| GenesisError::MessageBus {
                message: e.to_string(),
            })?;
            let topic = named(OUTCOMES_TOPIC).map_err(|e| GenesisError::MessageBus {
                message: e.to_string(),
            })?;

            // Persisted durability on the topic: an outcome lost to a broker
            // restart is a deployment stuck in a transient state with nothing
            // left to move it.
            bounded(
                "a publish",
                self.client.send_messages(
                    &stream,
                    &topic,
                    &Partitioning::partition_id(PARTITION),
                    &mut [IggyMessage::from(payload)],
                ),
            )
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to publish outcome: {e}"),
            })?;

            Ok(())
        })
    }
}
