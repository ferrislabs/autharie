//! The consumer against a real broker: what replaced RabbitMQ's per-message
//! ack and requeue has to hold on Iggy's single offset per consumer.
//!
//! Skipped unless `IGGY_URL` is set, for example
//! `IGGY_URL=iggy://autharie:autharie@127.0.0.1:8090`; set `REQUIRE_IGGY_URL=1`
//! to turn that skip into a failure, as CI does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autharie_iggy::iggy::prelude::{
    Consumer, Identifier, IggyClient, IggyMessage, MessageClient, Partitioning,
};
use autharie_iggy::{ACTIONS_TOPIC, DEFAULT_BUDGET, PARTITION, STREAM, named};
use chrono::Utc;
use genesis_core::application::dispatcher::EventDispatcher;
use genesis_core::domain::entities::action_event::ActionEvent;
use genesis_core::domain::error::GenesisError;
use genesis_core::domain::ports::{BoxFuture, EventConsumer, EventHandler};
use genesis_core::infrastructure::message_bus::consumer::IggyConsumer;
use uuid::Uuid;

fn url() -> Option<String> {
    match std::env::var("IGGY_URL") {
        Ok(url) => Some(url),
        Err(_) => {
            assert!(
                std::env::var("REQUIRE_IGGY_URL").is_err(),
                "REQUIRE_IGGY_URL is set but IGGY_URL is not"
            );
            eprintln!("IGGY_URL is unset: skipping");
            None
        }
    }
}

/// Records every delivery, and fails the actions it is told to fail for as
/// many attempts as it is told to.
#[derive(Default)]
struct Recorder {
    seen: Mutex<HashMap<Uuid, u32>>,
    failures: Mutex<HashMap<Uuid, u32>>,
}

impl Recorder {
    fn failing(action_id: Uuid, times: u32) -> Self {
        let recorder = Self::default();
        recorder.failures.lock().unwrap().insert(action_id, times);
        recorder
    }

    fn seen(&self, action_id: Uuid) -> u32 {
        self.seen
            .lock()
            .unwrap()
            .get(&action_id)
            .copied()
            .unwrap_or(0)
    }
}

impl EventHandler for Recorder {
    fn routing_key(&self) -> &str {
        "*"
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            *self
                .seen
                .lock()
                .unwrap()
                .entry(event.action_id)
                .or_insert(0) += 1;

            let mut failures = self.failures.lock().unwrap();
            match failures.get_mut(&event.action_id) {
                Some(left) if *left > 0 => {
                    *left -= 1;
                    Err(GenesisError::MessageBus {
                        message: "failing on purpose".into(),
                    })
                }
                _ => Ok(()),
            }
        })
    }
}

async fn client(url: &str) -> IggyClient {
    let client = autharie_iggy::connect_with_retry(url, DEFAULT_BUDGET)
        .await
        .unwrap();
    autharie_iggy::ensure_topic(&client, ACTIONS_TOPIC)
        .await
        .unwrap();
    client
}

/// Publishes an action and returns the offset it landed at.
async fn publish(client: &IggyClient, action_id: Uuid) -> u64 {
    let event = ActionEvent {
        action_id,
        deployment_id: Some(Uuid::new_v4()),
        dataplane_id: Uuid::new_v4(),
        routing_key: "deployment.create".into(),
        version: 1,
        payload: serde_json::json!({}),
        occurred_at: Utc::now(),
    };

    client
        .send_messages(
            &named(STREAM).unwrap(),
            &named(ACTIONS_TOPIC).unwrap(),
            &Partitioning::partition_id(PARTITION),
            &mut [IggyMessage::from(serde_json::to_vec(&event).unwrap())],
        )
        .await
        .unwrap();

    autharie_iggy::last_offset(client, ACTIONS_TOPIC)
        .await
        .unwrap()
        .unwrap()
}

async fn committed(client: &IggyClient, name: &str) -> Option<u64> {
    let consumer = Consumer::new(Identifier::named(name).unwrap());
    autharie_iggy::stored_offset(client, &consumer, ACTIONS_TOPIC)
        .await
        .unwrap()
}

fn start(url: &str, name: &str, recorder: Arc<Recorder>) -> tokio::task::JoinHandle<()> {
    let consumer = IggyConsumer::new(
        url,
        name,
        Arc::new(EventDispatcher::new(vec![
            recorder as Arc<dyn EventHandler>,
        ])),
    );

    tokio::spawn(async move {
        consumer.run().await.unwrap();
    })
}

async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn consumer_name() -> String {
    format!("genesis-test-{}", Uuid::new_v4().simple())
}

/// A failure is retried after its backoff, and does not hold up the action
/// behind it in the meantime.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_action_is_retried_without_holding_up_the_next() {
    let Some(url) = url() else { return };
    let client = client(&url).await;
    let (failing, next) = (Uuid::new_v4(), Uuid::new_v4());
    let recorder = Arc::new(Recorder::failing(failing, 1));
    let name = consumer_name();

    publish(&client, failing).await;
    let last = publish(&client, next).await;
    let consumer = start(&url, &name, recorder.clone());

    until("the next action", || recorder.seen(next) == 1).await;
    assert_eq!(
        recorder.seen(failing),
        1,
        "handled after the failure, before the retry came due"
    );

    until("the retry", || recorder.seen(failing) == 2).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    // At least: tests run in parallel on the one topic, so actions published
    // after `last` may be settled and committed too.
    let committed = committed(&client, &name).await;
    assert!(
        committed.is_some_and(|offset| offset >= last),
        "committed {committed:?}, short of {last}"
    );

    consumer.abort();
}

/// What an unacked RabbitMQ delivery did: a Genesis that stops while an action
/// waits for its retry gets it again when it starts.
#[tokio::test(flavor = "multi_thread")]
async fn an_action_waiting_for_its_retry_is_delivered_again_after_a_restart() {
    let Some(url) = url() else { return };
    let client = client(&url).await;
    let (failing, next) = (Uuid::new_v4(), Uuid::new_v4());
    let name = consumer_name();

    let failing_offset = publish(&client, failing).await;
    publish(&client, next).await;

    let first = Arc::new(Recorder::failing(failing, u32::MAX));
    let consumer = start(&url, &name, first.clone());
    until("the next action", || first.seen(next) == 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    consumer.abort();

    let committed = committed(&client, &name).await;
    assert!(
        committed.is_none_or(|offset| offset < failing_offset),
        "committed {committed:?}, past the action still waiting at {failing_offset}"
    );

    let second = Arc::new(Recorder::default());
    let consumer = start(&url, &name, second.clone());
    until("the redelivery", || second.seen(failing) == 1).await;

    consumer.abort();
}
