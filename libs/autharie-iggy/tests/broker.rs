//! Against a real broker. Skipped unless `IGGY_URL` is set, for example
//! `IGGY_URL=iggy://autharie:autharie@127.0.0.1:8090`; set
//! `REQUIRE_IGGY_URL=1` to turn that skip into a failure, as CI does.
//!
//! Each test declares a topic of its own. On a shared topic, tests running in
//! parallel publish between another test's write and its read, and a fresh
//! broker -- the one CI starts -- has an empty topic where a local one does
//! not.

use autharie_iggy::{
    DEFAULT_BUDGET, PARTITION, STREAM, bounded, connect_with_retry, ensure_topic, last_offset,
    named, stored_offset,
};
use iggy::prelude::*;
use uuid::Uuid;

/// A connected client and a topic no other test uses.
async fn setup() -> Option<(IggyClient, String)> {
    let Ok(url) = std::env::var("IGGY_URL") else {
        assert!(
            std::env::var("REQUIRE_IGGY_URL").is_err(),
            "REQUIRE_IGGY_URL is set but IGGY_URL is not"
        );
        eprintln!("IGGY_URL is unset: skipping");
        return None;
    };

    let client = connect_with_retry(&url, DEFAULT_BUDGET).await.unwrap();
    let topic = format!("test-{}", Uuid::new_v4().simple());
    ensure_topic(&client, &topic).await.unwrap();

    Some((client, topic))
}

fn consumer() -> Consumer {
    Consumer::new(Identifier::named(&format!("test-{}", Uuid::new_v4().simple())).unwrap())
}

async fn publish(client: &IggyClient, topic: &str, payload: &str) -> u64 {
    bounded(
        "a publish",
        client.send_messages(
            &named(STREAM).unwrap(),
            &named(topic).unwrap(),
            &Partitioning::partition_id(PARTITION),
            &mut [IggyMessage::from(payload.to_string())],
        ),
    )
    .await
    .unwrap();

    last_offset(client, topic).await.unwrap().unwrap()
}

async fn teardown(client: &IggyClient, topic: &str) {
    client
        .delete_topic(&named(STREAM).unwrap(), &named(topic).unwrap())
        .await
        .ok();
}

/// Every Herald and every Genesis declares on startup, so a second
/// declaration must find what the first made rather than fail.
#[tokio::test]
async fn declaring_twice_is_harmless() {
    let Some((client, topic)) = setup().await else {
        return;
    };

    ensure_topic(&client, &topic).await.unwrap();

    teardown(&client, &topic).await;
}

#[tokio::test]
async fn an_empty_topic_has_no_last_offset() {
    let Some((client, topic)) = setup().await else {
        return;
    };

    assert_eq!(last_offset(&client, &topic).await.unwrap(), None);

    teardown(&client, &topic).await;
}

#[tokio::test]
async fn a_published_message_is_read_back_at_its_offset() {
    let Some((client, topic)) = setup().await else {
        return;
    };
    let payload = Uuid::new_v4().to_string();

    let offset = publish(&client, &topic, &payload).await;
    let polled = bounded(
        "a poll",
        client.poll_messages(
            &named(STREAM).unwrap(),
            &named(&topic).unwrap(),
            Some(PARTITION),
            &consumer(),
            &PollingStrategy::offset(offset),
            1,
            false,
        ),
    )
    .await
    .unwrap();

    assert_eq!(offset, 0, "the first message of a fresh topic");
    assert_eq!(polled.messages.len(), 1);
    assert_eq!(polled.messages[0].header.offset, offset);
    assert_eq!(polled.messages[0].payload_as_string().unwrap(), payload);

    teardown(&client, &topic).await;
}

/// A consumer that never committed must read as one, not as one that
/// committed offset 0: the two start reading in different places.
#[tokio::test]
async fn a_consumer_that_never_committed_has_no_offset() {
    let Some((client, topic)) = setup().await else {
        return;
    };
    let consumer = consumer();

    assert_eq!(
        stored_offset(&client, &consumer, &topic).await.unwrap(),
        None
    );

    // The broker refuses an offset past the end of the partition, so the
    // message has to exist before its offset can be committed.
    let offset = publish(&client, &topic, "committed").await;
    bounded(
        "an offset store",
        client.store_consumer_offset(
            &consumer,
            &named(STREAM).unwrap(),
            &named(&topic).unwrap(),
            Some(PARTITION),
            offset,
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        stored_offset(&client, &consumer, &topic).await.unwrap(),
        Some(offset)
    );

    teardown(&client, &topic).await;
}
