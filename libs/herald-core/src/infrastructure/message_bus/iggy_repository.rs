use autharie_iggy::iggy::prelude::{IggyClient, IggyMessage, MessageClient, Partitioning};
use autharie_iggy::{ACTIONS_TOPIC, PARTITION, STREAM, bounded, named};

use crate::domain::entities::action::ActionEvent;
use crate::domain::error::HeraldError;
use crate::domain::ports::MessageBusRepository;

/// Publishes [`ActionEvent`]s to the actions topic, where Genesis reads them.
///
/// The routing key travels inside the event rather than beside it: Iggy routes
/// nothing, and Genesis decides from the key which actions are its own.
///
/// Ordering across actions of a single deployment is not guaranteed. Delivery
/// is at-least-once: a caller acks a claimed action with the control plane
/// only after a successful publish, so a lost ack simply causes the action to
/// be reclaimed and republished once its lease expires. Consumers must
/// deduplicate on `action_id`.
pub struct IggyMessageBusRepository {
    // One client for the life of the process: it reconnects and signs in
    // again on its own when the broker restarts or drops the connection.
    client: IggyClient,
}

impl IggyMessageBusRepository {
    /// Connects to the broker and declares the actions topic.
    ///
    /// Connecting here rather than lazily so a broker that is unreachable at
    /// startup still fails the pod, which is the one signal an operator has.
    pub async fn connect(url: &str) -> Result<Self, HeraldError> {
        let client = autharie_iggy::connect_with_retry(url, autharie_iggy::DEFAULT_BUDGET)
            .await
            .map_err(|err| HeraldError::MessageBus {
                message: format!("failed to reach the broker: {err}"),
            })?;

        autharie_iggy::ensure_topic(&client, ACTIONS_TOPIC)
            .await
            .map_err(|err| HeraldError::MessageBus {
                message: format!("failed to declare the '{ACTIONS_TOPIC}' topic: {err}"),
            })?;

        Ok(Self { client })
    }
}

impl MessageBusRepository for IggyMessageBusRepository {
    async fn publish(&self, event: ActionEvent) -> Result<(), HeraldError> {
        let action_id = event.action_id;

        let payload = serde_json::to_vec(&event).map_err(|err| HeraldError::Internal {
            message: format!("failed to serialize action event {action_id}: {err}"),
        })?;

        let stream = named(STREAM).map_err(bus_error)?;
        let topic = named(ACTIONS_TOPIC).map_err(bus_error)?;

        // Returns once the broker has the message on disk: the topic is
        // declared with persisted durability, so a success here survives a
        // broker restart.
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
        .map_err(|err| HeraldError::MessageBus {
            message: format!("failed to publish action event {action_id}: {err}"),
        })?;

        Ok(())
    }
}

fn bus_error(err: autharie_iggy::Error) -> HeraldError {
    HeraldError::MessageBus {
        message: err.to_string(),
    }
}
