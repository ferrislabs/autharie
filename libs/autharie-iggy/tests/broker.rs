//! Against a real broker. Skipped unless `IGGY_URL` is set, for example
//! `IGGY_URL=iggy://autharie:autharie@127.0.0.1:8090`; set
//! `REQUIRE_IGGY_URL=1` to turn that skip into a failure, as CI does.

use autharie_iggy::{
    ACTIONS_TOPIC, DEFAULT_BUDGET, PARTITION, STREAM, bounded, connect_with_retry, ensure_topic,
    last_offset, named, stored_offset,
};
use iggy::prelude::*;
use uuid::Uuid;

async fn client() -> Option<IggyClient> {
    let Ok(url) = std::env::var("IGGY_URL") else {
        assert!(
            std::env::var("REQUIRE_IGGY_URL").is_err(),
            "REQUIRE_IGGY_URL is set but IGGY_URL is not"
        );
        eprintln!("IGGY_URL is unset: skipping");
        return None;
    };

    let client = connect_with_retry(&url, DEFAULT_BUDGET).await.unwrap();
    ensure_topic(&client, ACTIONS_TOPIC).await.unwrap();

    Some(client)
}

fn consumer() -> Consumer {
    Consumer::new(Identifier::named(&format!("test-{}", Uuid::new_v4().simple())).unwrap())
}

/// Every Herald and every Genesis declares on startup, so a second
/// declaration must find what the first made rather than fail.
#[tokio::test]
async fn declaring_twice_is_harmless() {
    let Some(client) = client().await else { return };

    ensure_topic(&client, ACTIONS_TOPIC).await.unwrap();
}

#[tokio::test]
async fn a_published_message_is_read_back_at_its_offset() {
    let Some(client) = client().await else { return };
    let payload = Uuid::new_v4().to_string();

    bounded(
        "a publish",
        client.send_messages(
            &named(STREAM).unwrap(),
            &named(ACTIONS_TOPIC).unwrap(),
            &Partitioning::partition_id(PARTITION),
            &mut [IggyMessage::from(payload.clone())],
        ),
    )
    .await
    .unwrap();

    let offset = last_offset(&client, ACTIONS_TOPIC).await.unwrap().unwrap();
    let polled = bounded(
        "a poll",
        client.poll_messages(
            &named(STREAM).unwrap(),
            &named(ACTIONS_TOPIC).unwrap(),
            Some(PARTITION),
            &consumer(),
            &PollingStrategy::offset(offset),
            1,
            false,
        ),
    )
    .await
    .unwrap();

    assert_eq!(polled.messages.len(), 1);
    assert_eq!(polled.messages[0].header.offset, offset);
    assert_eq!(polled.messages[0].payload_as_string().unwrap(), payload);
}

/// A consumer that never committed must read as one, not as one that
/// committed offset 0: the two start reading in different places.
#[tokio::test]
async fn a_consumer_that_never_committed_has_no_offset() {
    let Some(client) = client().await else { return };
    let consumer = consumer();

    assert_eq!(
        stored_offset(&client, &consumer, ACTIONS_TOPIC)
            .await
            .unwrap(),
        None
    );

    bounded(
        "an offset store",
        client.store_consumer_offset(
            &consumer,
            &named(STREAM).unwrap(),
            &named(ACTIONS_TOPIC).unwrap(),
            Some(PARTITION),
            0,
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        stored_offset(&client, &consumer, ACTIONS_TOPIC)
            .await
            .unwrap(),
        Some(0)
    );
}
