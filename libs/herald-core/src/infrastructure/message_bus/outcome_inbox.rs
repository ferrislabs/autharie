use autharie_iggy::iggy::prelude::{
    Consumer, ConsumerOffsetClient, Identifier, IggyClient, MessageClient, PollingStrategy,
};
use autharie_iggy::{OUTCOMES_TOPIC, PARTITION, STREAM, bounded, named};
use serde_json::from_slice;
use tracing::{info, warn};

use crate::domain::entities::outcome::DeploymentOutcomeReport;
use crate::domain::error::HeraldError;
use crate::domain::ports::OutcomeInboxRepository;

/// Reads what Genesis published on the outcomes topic.
///
/// Each Herald reads under a consumer name of its own, so every one of them
/// sees every outcome -- what the queue per shard bound to `outcome.#` did on
/// RabbitMQ -- and the broker remembers how far each one got.
pub struct IggyOutcomeInbox {
    // One client for the life of the process: it reconnects and signs in
    // again on its own when the broker restarts or drops the connection.
    client: IggyClient,
    consumer: Consumer,
}

impl IggyOutcomeInbox {
    /// Connects, declares the outcomes topic, and places a consumer reading
    /// for the first time at the end of what is already there.
    ///
    /// At the end rather than the start because that is what a queue does: it
    /// receives what is published after it exists. Starting at the beginning
    /// would replay a week of outcomes -- statuses long since superseded -- on
    /// the first start of every new shard.
    pub async fn connect(url: &str, consumer: &str) -> Result<Self, HeraldError> {
        let client = autharie_iggy::connect_with_retry(url, autharie_iggy::DEFAULT_BUDGET)
            .await
            .map_err(|err| HeraldError::MessageBus {
                message: format!("failed to reach the broker: {err}"),
            })?;

        autharie_iggy::ensure_topic(&client, OUTCOMES_TOPIC)
            .await
            .map_err(|err| HeraldError::MessageBus {
                message: format!("failed to declare the '{OUTCOMES_TOPIC}' topic: {err}"),
            })?;

        let consumer =
            Consumer::new(
                Identifier::named(consumer).map_err(|err| HeraldError::MessageBus {
                    message: format!("'{consumer}' cannot name a consumer: {err}"),
                })?,
            );

        let inbox = Self { client, consumer };
        inbox.start_at_the_end().await?;

        Ok(inbox)
    }

    async fn start_at_the_end(&self) -> Result<(), HeraldError> {
        let stored = autharie_iggy::stored_offset(&self.client, &self.consumer, OUTCOMES_TOPIC)
            .await
            .map_err(bus_error)?;

        if stored.is_some() {
            return Ok(());
        }

        // Nothing published yet: the first message to arrive is this
        // consumer's first, so there is nothing to skip.
        let Some(last) = autharie_iggy::last_offset(&self.client, OUTCOMES_TOPIC)
            .await
            .map_err(bus_error)?
        else {
            return Ok(());
        };

        bounded(
            "an offset store",
            self.client.store_consumer_offset(
                &self.consumer,
                &named(STREAM).map_err(bus_error)?,
                &named(OUTCOMES_TOPIC).map_err(bus_error)?,
                Some(PARTITION),
                last,
            ),
        )
        .await
        .map_err(bus_error)?;

        info!(
            skipped_through = last,
            "reading outcomes from the end of the topic"
        );

        Ok(())
    }
}

impl OutcomeInboxRepository for IggyOutcomeInbox {
    async fn drain(&self, limit: usize) -> Result<Vec<DeploymentOutcomeReport>, HeraldError> {
        let count = u32::try_from(limit).unwrap_or(u32::MAX);
        if count == 0 {
            return Ok(Vec::new());
        }

        // Committed as it is read, deliberately. The alternative is holding
        // the offset until the control plane confirms, which turns one
        // unreachable control plane into a topic that never drains. A lost
        // report costs a status that stays stale; a stuck inbox costs every
        // report behind it.
        let polled = bounded(
            "a poll",
            self.client.poll_messages(
                &named(STREAM).map_err(bus_error)?,
                &named(OUTCOMES_TOPIC).map_err(bus_error)?,
                Some(PARTITION),
                &self.consumer,
                &PollingStrategy::next(),
                count,
                true,
            ),
        )
        .await
        .map_err(|err| HeraldError::MessageBus {
            message: format!("failed to read the outcomes topic: {err}"),
        })?;

        let mut reports = Vec::with_capacity(polled.messages.len());

        for message in polled.messages {
            match from_slice::<DeploymentOutcomeReport>(&message.payload) {
                Ok(report) => reports.push(report),
                // Skipped, not retried: a message this Herald cannot parse
                // will not parse on the next attempt either.
                Err(err) => warn!(
                    %err,
                    offset = message.header.offset,
                    "discarding an unreadable outcome"
                ),
            }
        }

        Ok(reports)
    }
}

fn bus_error(err: autharie_iggy::Error) -> HeraldError {
    HeraldError::MessageBus {
        message: err.to_string(),
    }
}
