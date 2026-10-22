use crate::application::dispatcher::EventDispatcher;
use crate::domain::entities::action_event::ActionEvent;
use crate::domain::error::GenesisError;
use crate::domain::ports::EventConsumer;
use autharie_iggy::iggy::prelude::{
    Consumer, ConsumerOffsetClient, Identifier, IggyClient, MessageClient, PollingStrategy,
};
use autharie_iggy::{ACTIONS_TOPIC, DEFAULT_BUDGET, PARTITION, STREAM, bounded, named};
use serde_json::from_slice;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// The routing keys Genesis acts on: deployment lifecycle events and the
/// actions that target a whole data plane. An action whose key no pattern here
/// matches is published by Herald and skipped here.
pub const BINDING_KEYS: [&str; 2] = ["deployment.#", "dataplane.#"];

/// Whether Genesis acts on `routing_key`, under the topic rules RabbitMQ used
/// to apply for it: words are separated by dots, `*` stands for one word and
/// `#` for any number of them.
pub fn is_bound(routing_key: &str) -> bool {
    let words: Vec<&str> = routing_key.split('.').collect();
    BINDING_KEYS
        .iter()
        .any(|pattern| matches_pattern(&pattern.split('.').collect::<Vec<_>>(), &words))
}

fn matches_pattern(pattern: &[&str], words: &[&str]) -> bool {
    match pattern.split_first() {
        None => words.is_empty(),
        Some((&"#", rest)) => (0..=words.len()).any(|taken| matches_pattern(rest, &words[taken..])),
        Some((&"*", rest)) => !words.is_empty() && matches_pattern(rest, &words[1..]),
        Some((word, rest)) => words.first() == Some(word) && matches_pattern(rest, &words[1..]),
    }
}

/// Shortest wait before a failed event is tried again.
const RETRY_BASE: Duration = Duration::from_secs(1);

/// Longest wait between attempts.
///
/// The failures worth retrying are ones that clear on their own -- a namespace
/// mid-creation, an API server restarting, a webhook not yet ready. Half a
/// minute is long enough to be quiet and short enough that a deployment
/// recovers without anyone watching.
const RETRY_MAX: Duration = Duration::from_secs(30);

/// How long to wait before trying a failed event again.
///
/// Doubling from `RETRY_BASE`, capped at `RETRY_MAX`. Retrying immediately --
/// which is what this once did -- turns a failure that will never clear into a
/// hot loop: eighteen attempts a second, each writing two log lines, for as
/// long as the process lives. A namespace that did not exist produced exactly
/// that, and buried every other line in the log.
fn retry_delay(attempt: u32) -> Duration {
    RETRY_BASE
        .saturating_mul(2_u32.saturating_pow(attempt.min(16)))
        .min(RETRY_MAX)
}

/// How many messages one poll asks for.
const BATCH: u32 = 100;

/// How long to wait before polling again when the last poll found nothing.
///
/// Iggy has no push to a plain consumer: an idle Genesis asks. Half a second
/// is far below Herald's own cycle, so it adds nothing a user would notice.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Which messages are settled, and so how far the broker may be told this
/// consumer got.
///
/// Iggy keeps one offset per consumer, not an ack per message. A message that
/// failed and waits for its retry must not be skipped by a later one that
/// succeeded, so the offset committed stops just before the oldest message
/// still waiting. If Genesis stops, it starts again from there: the waiting
/// message is delivered again -- what an unacked RabbitMQ delivery did -- and
/// so are the settled ones after it, which handlers already treat as the
/// redeliveries at-least-once allows.
#[derive(Debug)]
struct Progress {
    /// The offset the next poll starts at.
    next: u64,
    /// Offsets read and failed, whose retry has not succeeded yet.
    waiting: BTreeSet<u64>,
    /// The last offset the broker was told about.
    committed: Option<u64>,
}

impl Progress {
    /// Continuing after what was committed, or from the first message ever
    /// published. From the first rather than the end: an action published
    /// before Genesis ever ran is still Genesis' to act on.
    fn after(committed: Option<u64>) -> Self {
        Self {
            next: committed.map_or(0, |offset| offset.saturating_add(1)),
            waiting: BTreeSet::new(),
            committed,
        }
    }

    fn read(&mut self, offset: u64) {
        self.next = self.next.max(offset.saturating_add(1));
    }

    fn failed(&mut self, offset: u64) {
        self.waiting.insert(offset);
    }

    fn settled(&mut self, offset: u64) {
        self.waiting.remove(&offset);
    }

    /// The offset to commit now, if it is further than the last one.
    fn to_commit(&self) -> Option<u64> {
        let through = match self.waiting.first() {
            Some(&oldest) => oldest.checked_sub(1),
            None => self.next.checked_sub(1),
        }?;

        match self.committed {
            Some(committed) if committed >= through => None,
            _ => Some(through),
        }
    }

    fn committed(&mut self, offset: u64) {
        self.committed = Some(offset);
    }
}

/// The state one run of the consumer carries between messages.
struct Worker<'a> {
    dispatcher: &'a EventDispatcher,
    progress: Progress,
    // Per action, in memory, as RabbitMQ never counted them either. Losing
    // the counts on restart is fine -- the backoff restarts from the bottom,
    // which is the same place a fresh process would start anyway.
    attempts: HashMap<Uuid, u32>,
    retries: mpsc::UnboundedSender<(u64, ActionEvent)>,
}

impl Worker<'_> {
    async fn handle(&mut self, offset: u64, event: ActionEvent) {
        let action_id = event.action_id;

        match self.dispatcher.dispatch(event.clone()).await {
            Ok(()) => {
                self.attempts.remove(&action_id);
                self.progress.settled(offset);
            }
            Err(e) => {
                let attempt = self.attempts.entry(action_id).or_insert(0);
                *attempt = attempt.saturating_add(1);
                let delay = retry_delay(*attempt);

                warn!(
                    %action_id,
                    attempt = *attempt,
                    retry_in_seconds = delay.as_secs(),
                    "handler error, retrying: {e}"
                );

                self.progress.failed(offset);

                // Spawned, not awaited. Waiting here would hold the consumer
                // for the whole delay, so one event that keeps failing would
                // throttle every other event behind it to the same cadence --
                // turning a backoff on one deployment into a stall on all of
                // them.
                let retries = self.retries.clone();
                tokio::spawn(async move {
                    sleep(delay).await;
                    retries.send((offset, event)).ok();
                });
            }
        }
    }
}

pub struct IggyConsumer {
    url: String,
    consumer: String,
    dispatcher: Arc<EventDispatcher>,
}

impl IggyConsumer {
    /// `consumer` names this reader to the broker, which keeps how far it got
    /// under that name.
    pub fn new(
        url: impl Into<String>,
        consumer: impl Into<String>,
        dispatcher: Arc<EventDispatcher>,
    ) -> Self {
        Self {
            url: url.into(),
            consumer: consumer.into(),
            dispatcher,
        }
    }

    async fn connect(&self) -> Result<(IggyClient, Consumer), GenesisError> {
        let client = autharie_iggy::connect_with_retry(&self.url, DEFAULT_BUDGET)
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to reach the broker: {e}"),
            })?;

        autharie_iggy::ensure_topic(&client, ACTIONS_TOPIC)
            .await
            .map_err(|e| GenesisError::MessageBus {
                message: format!("failed to declare the '{ACTIONS_TOPIC}' topic: {e}"),
            })?;

        let consumer = Consumer::new(Identifier::named(&self.consumer).map_err(|e| {
            GenesisError::MessageBus {
                message: format!("'{}' cannot name a consumer: {e}", self.consumer),
            }
        })?);

        Ok((client, consumer))
    }
}

impl EventConsumer for IggyConsumer {
    async fn run(&self) -> Result<(), GenesisError> {
        let (client, consumer) = self.connect().await?;
        let stream = named(STREAM).map_err(bus_error)?;
        let topic = named(ACTIONS_TOPIC).map_err(bus_error)?;

        let committed = autharie_iggy::stored_offset(&client, &consumer, ACTIONS_TOPIC)
            .await
            .map_err(bus_error)?;

        let (retries, mut due) = mpsc::unbounded_channel();
        let mut worker = Worker {
            dispatcher: &self.dispatcher,
            progress: Progress::after(committed),
            attempts: HashMap::new(),
            retries,
        };

        info!(
            consumer = %self.consumer,
            from_offset = worker.progress.next,
            "consuming actions"
        );

        let mut failing_since: Option<Instant> = None;

        loop {
            while let Ok((offset, event)) = due.try_recv() {
                worker.handle(offset, event).await;
            }

            let polled = bounded(
                "a poll",
                client.poll_messages(
                    &stream,
                    &topic,
                    Some(PARTITION),
                    &consumer,
                    &PollingStrategy::offset(worker.progress.next),
                    BATCH,
                    false,
                ),
            )
            .await;

            let messages = match polled {
                Ok(polled) => {
                    failing_since = None;
                    polled.messages
                }
                Err(e) => {
                    // The client reconnects on its own, so a failed poll is
                    // usually a broker coming back. One that stays away past
                    // the budget is a Genesis that has gone deaf, and the pod
                    // exiting is what tells someone.
                    let since = *failing_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= DEFAULT_BUDGET {
                        return Err(GenesisError::MessageBus {
                            message: format!(
                                "the broker has not answered a poll since {:?}: {e}",
                                since.elapsed()
                            ),
                        });
                    }

                    warn!("failed to poll for actions: {e}");
                    sleep(POLL_INTERVAL).await;
                    continue;
                }
            };

            let idle = messages.is_empty();

            for message in messages {
                let offset = message.header.offset;
                worker.progress.read(offset);

                match from_slice::<ActionEvent>(&message.payload) {
                    // Skipped, not retried: an event that does not parse will
                    // not parse on the next attempt either.
                    Err(e) => error!(offset, "failed to deserialize event: {e}"),
                    Ok(event) if !is_bound(&event.routing_key) => {
                        debug!(offset, routing_key = %event.routing_key, "not an action for Genesis");
                    }
                    Ok(event) => worker.handle(offset, event).await,
                }
            }

            if let Some(offset) = worker.progress.to_commit() {
                match bounded(
                    "an offset store",
                    client.store_consumer_offset(
                        &consumer,
                        &stream,
                        &topic,
                        Some(PARTITION),
                        offset,
                    ),
                )
                .await
                {
                    Ok(()) => worker.progress.committed(offset),
                    // Tried again after the next poll. Until then a restart
                    // redelivers a little more than it needs to, nothing worse.
                    Err(e) => warn!(offset, "failed to commit the consumer offset: {e}"),
                }
            }

            if idle {
                tokio::select! {
                    () = sleep(POLL_INTERVAL) => {}
                    Some((offset, event)) = due.recv() => worker.handle(offset, event).await,
                }
            }
        }
    }
}

fn bus_error(err: autharie_iggy::Error) -> GenesisError {
    GenesisError::MessageBus {
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Progress, RETRY_BASE, RETRY_MAX, is_bound, matches_pattern, retry_delay};

    #[test]
    fn the_first_retry_waits_the_base_delay() {
        assert_eq!(retry_delay(1), RETRY_BASE * 2);
    }

    #[test]
    fn the_delay_doubles_until_it_reaches_the_cap() {
        assert!(retry_delay(1) < retry_delay(2));
        assert!(retry_delay(2) < retry_delay(3));
        assert_eq!(retry_delay(20), RETRY_MAX);
    }

    /// The bug this exists for. An immediate requeue turned a failure that
    /// would never clear into eighteen attempts a second, each writing two log
    /// lines, for as long as the process lived.
    #[test]
    fn no_attempt_ever_requeues_immediately() {
        for attempt in 0..64 {
            assert!(
                retry_delay(attempt) >= RETRY_BASE,
                "attempt {attempt} waited less than the base delay"
            );
        }
    }

    /// `2^attempt` overflows a u32 long before attempt 64, and a panic in the
    /// consumer loop would take Genesis down over a retry.
    #[test]
    fn a_large_attempt_count_saturates_rather_than_overflowing() {
        assert_eq!(retry_delay(u32::MAX), RETRY_MAX);
    }

    #[test]
    fn deployment_and_data_plane_actions_are_bound() {
        assert!(is_bound("deployment.create"));
        assert!(is_bound("deployment.upgrade"));
        assert!(is_bound("dataplane.upgrade"));
    }

    #[test]
    fn the_agents_action_is_bound() {
        assert!(is_bound("deployment.agents"));
    }

    #[test]
    fn the_webhook_actions_are_bound() {
        assert!(is_bound("deployment.webhooks"));
        assert!(is_bound("deployment.webhook.rotate_secret"));
        assert!(is_bound("deployment.webhook.retry_delivery"));
    }

    #[test]
    fn a_realm_action_is_no_longer_bound() {
        assert!(!is_bound("realm.create"));
    }

    #[test]
    fn outcomes_and_unknown_namespaces_are_not_bound() {
        assert!(!is_bound("outcome.deployment"));
        assert!(!is_bound("backup.create"));
        assert!(!is_bound("dataplanes.upgrade"));
    }

    #[test]
    fn hash_matches_no_word_and_star_exactly_one() {
        assert!(matches_pattern(&["a", "#"], &["a"]));
        assert!(matches_pattern(&["a", "*"], &["a", "b"]));
        assert!(!matches_pattern(&["a", "*"], &["a"]));
        assert!(!matches_pattern(&["a", "*"], &["a", "b", "c"]));
    }

    #[test]
    fn a_consumer_that_never_committed_starts_at_the_first_message() {
        let progress = Progress::after(None);

        assert_eq!(progress.next, 0);
        assert_eq!(
            progress.to_commit(),
            None,
            "nothing read, nothing to commit"
        );
    }

    #[test]
    fn a_consumer_resumes_after_what_it_committed() {
        assert_eq!(Progress::after(Some(41)).next, 42);
    }

    #[test]
    fn everything_read_and_settled_is_committed() {
        let mut progress = Progress::after(None);
        for offset in 0..3 {
            progress.read(offset);
            progress.settled(offset);
        }

        assert_eq!(progress.to_commit(), Some(2));
    }

    /// The point of the type. A later success must not carry the offset past
    /// a message still waiting for its retry, or a restart in that window
    /// loses the message.
    #[test]
    fn the_commit_stops_before_a_message_waiting_for_its_retry() {
        let mut progress = Progress::after(Some(9));
        for offset in 10..=12 {
            progress.read(offset);
        }
        progress.failed(11);
        progress.settled(10);
        progress.settled(12);

        assert_eq!(progress.to_commit(), Some(10));

        progress.committed(10);
        assert_eq!(progress.to_commit(), None, "no further than last time");

        progress.settled(11);
        assert_eq!(progress.to_commit(), Some(12));
    }

    #[test]
    fn a_failure_on_the_first_message_ever_commits_nothing() {
        let mut progress = Progress::after(None);
        progress.read(0);
        progress.failed(0);

        assert_eq!(progress.to_commit(), None);
    }

    /// A retry re-reads nothing: the offset the next poll starts at never
    /// moves back.
    #[test]
    fn reading_never_moves_the_next_poll_backwards() {
        let mut progress = Progress::after(None);
        progress.read(5);
        progress.read(3);

        assert_eq!(progress.next, 6);
    }
}
