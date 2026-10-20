//! The message bus between Herald and Genesis, on Apache Iggy.
//!
//! One stream, two topics, one partition each:
//!
//! - `actions`: Herald publishes the actions it claims from the control plane,
//!   Genesis consumes them.
//! - `outcomes`: Genesis publishes what became of a deployment, every Herald
//!   drains them.
//!
//! A topic per direction rather than RabbitMQ's one exchange split by routing
//! key: Iggy routes nothing, and two topics are what keep an outcome from ever
//! reaching Genesis as if it were an action. One partition, because the volume
//! is a handful of messages a minute and a single partition keeps the order
//! they were published in.
//!
//! Herald and Genesis start beside the broker in the same chart. On a fresh
//! install it is not accepting connections yet, which is an expected state
//! during startup. One that is still not up after the budget is a real failure,
//! and exiting then is right -- so the wait is bounded rather than infinite.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use iggy::prelude::*;
use tracing::info;

// Re-exported so Herald and Genesis use the one pinned SDK this crate is built
// against, rather than each naming a version of their own.
pub use iggy;

/// The stream both topics live in.
pub const STREAM: &str = "autharie";

/// Herald to Genesis: the actions claimed from the control plane.
pub const ACTIONS_TOPIC: &str = "actions";

/// Genesis to Herald: what became of a deployment.
pub const OUTCOMES_TOPIC: &str = "outcomes";

/// The one partition of each topic. Iggy numbers partitions from zero.
pub const PARTITION: u32 = 0;

/// How long to keep trying to reach the broker at startup before giving up.
///
/// Long enough for a broker that is slow to start -- a cold image pull, a busy
/// node -- and short enough that a genuinely unreachable broker still surfaces
/// as a failed pod rather than one that hangs looking healthy.
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(120);

/// How long one request to the broker may take once connected.
///
/// The client reconnects on its own and without a limit, so a request made
/// while the broker is away waits for it rather than failing. Bounded here so
/// that waiting turns into an error the caller can report: Herald's cycle and
/// Genesis' loop must keep turning, not hang on a broker that is gone.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a message is kept once published.
///
/// Far past any lease: an action nobody consumed within a week has long been
/// re-leased by the control plane, and keeping it only costs disk.
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Small segments, so retention frees space: only a sealed segment can expire,
/// and the default of 1 GiB would never seal at this volume.
const SEGMENT_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    Iggy(IggyError),
    /// The broker did not answer `what` within the time allowed.
    TimedOut {
        what: &'static str,
        after: Duration,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Iggy(err) => write!(f, "{err}"),
            Self::TimedOut { what, after } => {
                write!(f, "the broker did not answer {what} within {after:?}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<IggyError> for Error {
    fn from(err: IggyError) -> Self {
        Self::Iggy(err)
    }
}

/// Runs one request to the broker, failing it after [`OPERATION_TIMEOUT`].
pub async fn bounded<T>(
    what: &'static str,
    request: impl Future<Output = Result<T, IggyError>>,
) -> Result<T, Error> {
    within(what, OPERATION_TIMEOUT, request).await
}

async fn within<T>(
    what: &'static str,
    after: Duration,
    request: impl Future<Output = Result<T, IggyError>>,
) -> Result<T, Error> {
    match tokio::time::timeout(after, request).await {
        Ok(result) => result.map_err(Error::from),
        Err(_) => Err(Error::TimedOut { what, after }),
    }
}

/// The identifier of a stream or topic by its name.
pub fn named(name: &str) -> Result<Identifier, Error> {
    Ok(Identifier::named(name)?)
}

/// Connects to the broker, waiting up to `budget` for it to answer.
///
/// `url` is an Iggy connection string, `iggy://user:password@host:port`. The
/// client signs in on connect and again after every reconnect, so a broker
/// that restarts is picked up without the caller doing anything.
pub async fn connect_with_retry(url: &str, budget: Duration) -> Result<IggyClient, Error> {
    let client = IggyClient::from_connection_string(url)?;

    within("the connection", budget, client.connect()).await?;
    info!("connected to the broker");

    Ok(client)
}

/// Declares the stream and `topic`, unless they already exist.
///
/// Both sides declare both, because whichever starts first must find them.
/// Losing the race to create one is the same as finding it there.
pub async fn ensure_topic(client: &IggyClient, topic: &str) -> Result<(), Error> {
    let stream_id = named(STREAM)?;

    if bounded("a stream lookup", client.get_stream(&stream_id))
        .await?
        .is_none()
    {
        match bounded("a stream creation", client.create_stream(STREAM)).await {
            Ok(_) | Err(Error::Iggy(IggyError::StreamNameAlreadyExists(_))) => {}
            Err(err) => return Err(err),
        }
    }

    let topic_id = named(topic)?;

    if bounded("a topic lookup", client.get_topic(&stream_id, &topic_id))
        .await?
        .is_none()
    {
        let options = topic_options();

        match bounded(
            "a topic creation",
            client.create_topic(&stream_id, topic, &options),
        )
        .await
        {
            Ok(_) | Err(Error::Iggy(IggyError::TopicNameAlreadyExists(..))) => {}
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn topic_options() -> TopicCreateOptions {
    TopicCreateOptions {
        partitions_count: Some(1),
        message_expiry: Some(IggyExpiry::ExpireDuration(RETENTION.into())),
        segment_size: Some(IggyByteSize::from(SEGMENT_SIZE)),
        // Persisted, both of them: an action or an outcome lost to a broker
        // restart is a deployment stuck in a transient state with nothing left
        // to move it, and a consumer offset lost the same way replays a week.
        durability: Durability::Persisted,
        consumer_offset_durability: Durability::Persisted,
        preallocate_segments: Some(false),
        ..Default::default()
    }
}

/// The last offset `consumer` committed on `topic`, if it ever committed one.
pub async fn stored_offset(
    client: &IggyClient,
    consumer: &Consumer,
    topic: &str,
) -> Result<Option<u64>, Error> {
    let info = bounded(
        "an offset lookup",
        client.get_consumer_offset(consumer, &named(STREAM)?, &named(topic)?, Some(PARTITION)),
    )
    .await?;

    Ok(info.map(|info| info.stored_offset))
}

/// The offset of the last message in `topic`, if it holds any.
pub async fn last_offset(client: &IggyClient, topic: &str) -> Result<Option<u64>, Error> {
    let details = bounded(
        "a topic lookup",
        client.get_topic(&named(STREAM)?, &named(topic)?),
    )
    .await?;

    Ok(details
        .and_then(|details| {
            details
                .partitions
                .into_iter()
                .find(|partition| partition.id == PARTITION)
        })
        .filter(|partition| partition.messages_count > 0)
        .map(|partition| partition.current_offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The budget exists so a genuinely unreachable broker still fails. The
    /// client retries without a limit on its own, so without the budget a
    /// misconfiguration would be a pod that looks healthy and never does any
    /// work.
    #[tokio::test(start_paused = true)]
    async fn gives_up_once_the_budget_is_spent() {
        let started = tokio::time::Instant::now();

        let result =
            connect_with_retry("iggy://iggy:iggy@127.0.0.1:1", Duration::from_secs(3)).await;

        assert!(result.is_err(), "nothing is listening on that port");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "gave up late: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_request_that_never_answers_times_out() {
        let result = within(
            "a test",
            Duration::from_millis(10),
            std::future::pending::<Result<(), IggyError>>(),
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::TimedOut { what: "a test", .. })
        ));
    }

    #[test]
    fn a_malformed_url_is_refused_before_connecting() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let result = runtime.block_on(connect_with_retry("amqp://guest@host", DEFAULT_BUDGET));

        assert!(matches!(result, Err(Error::Iggy(_))));
    }
}
