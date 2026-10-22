//! The outcome inbox against a real broker.
//!
//! Skipped unless `IGGY_URL` is set, for example
//! `IGGY_URL=iggy://autharie:autharie@127.0.0.1:8090`; set `REQUIRE_IGGY_URL=1`
//! to turn that skip into a failure, as CI does.

use autharie_iggy::iggy::prelude::{IggyClient, IggyMessage, MessageClient, Partitioning};
use autharie_iggy::{DEFAULT_BUDGET, OUTCOMES_TOPIC, PARTITION, STREAM, named};
use herald_core::domain::ports::OutcomeInboxRepository;
use herald_core::infrastructure::message_bus::outcome_inbox::IggyOutcomeInbox;
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

async fn publish(client: &IggyClient, deployment_id: Uuid) {
    let report = serde_json::json!({ "deployment_id": deployment_id, "outcome": "ready" });

    client
        .send_messages(
            &named(STREAM).unwrap(),
            &named(OUTCOMES_TOPIC).unwrap(),
            &Partitioning::partition_id(PARTITION),
            &mut [IggyMessage::from(serde_json::to_vec(&report).unwrap())],
        )
        .await
        .unwrap();
}

fn shard() -> String {
    format!("herald.outcomes.test-{}", Uuid::new_v4().simple())
}

/// A new shard reads what is published after it starts, as a freshly bound
/// queue did -- not a week of outcomes long since superseded.
#[tokio::test]
async fn a_new_inbox_starts_at_the_end_of_the_topic() {
    let Some(url) = url() else { return };
    let client = autharie_iggy::connect_with_retry(&url, DEFAULT_BUDGET)
        .await
        .unwrap();
    autharie_iggy::ensure_topic(&client, OUTCOMES_TOPIC)
        .await
        .unwrap();

    let (before, after) = (Uuid::new_v4(), Uuid::new_v4());
    publish(&client, before).await;

    let inbox = IggyOutcomeInbox::connect(&url, &shard()).await.unwrap();
    publish(&client, after).await;

    let seen: Vec<Uuid> = inbox
        .drain(1000)
        .await
        .unwrap()
        .into_iter()
        .map(|report| report.deployment_id())
        .collect();

    assert!(
        !seen.contains(&before),
        "replayed an outcome from before it started"
    );
    assert!(seen.contains(&after));
}

/// Each shard sees every outcome, and a drained outcome is not drained again.
#[tokio::test]
async fn every_shard_sees_each_outcome_once() {
    let Some(url) = url() else { return };
    let client = autharie_iggy::connect_with_retry(&url, DEFAULT_BUDGET)
        .await
        .unwrap();
    autharie_iggy::ensure_topic(&client, OUTCOMES_TOPIC)
        .await
        .unwrap();

    let first = IggyOutcomeInbox::connect(&url, &shard()).await.unwrap();
    let second = IggyOutcomeInbox::connect(&url, &shard()).await.unwrap();

    let deployment = Uuid::new_v4();
    publish(&client, deployment).await;

    for inbox in [&first, &second] {
        let seen = inbox.drain(1000).await.unwrap();
        assert!(
            seen.iter()
                .any(|report| report.deployment_id() == deployment)
        );

        let again = inbox.drain(1000).await.unwrap();
        assert!(
            !again
                .iter()
                .any(|report| report.deployment_id() == deployment)
        );
    }
}
