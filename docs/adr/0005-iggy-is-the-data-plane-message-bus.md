# 0005. Apache Iggy carries actions and outcomes inside a data plane

Status: proposed
Date: 2026-10-20

## Context
Inside a data plane, Herald publishes the actions it claims from the control plane and Genesis consumes them; Genesis publishes outcomes back and every Herald drains them. This ran on a bundled RabbitMQ with no persistence, so a broker restart lost every queued action and outcome. RabbitMQ is also the heaviest component of a data plane (a 256 MiB request) for a few messages a minute, and the only one not written in Rust.

## Decision
A single-node Apache Iggy, server 0.9.0, with the Rust SDK pinned to the matching 0.11.0 in `libs/autharie-iggy`. One stream `autharie` holds two topics of one partition each: `actions` (Herald to Genesis) and `outcomes` (Genesis to Herald). The topics are declared with persisted durability, so a publish returns once the message is on disk. The bundled broker runs on a volume.

Iggy has no routing, no per-message ack and no requeue, so three things moved into the clients:
- Routing: Genesis reads the whole `actions` topic and skips keys outside `BINDING_KEYS`, with the same `is_bound` function the bindings were tested with.
- Retry: a failed action is retried in memory with the existing backoff. Genesis commits its offset only up to just before the oldest action still waiting, so a restart redelivers it, as an unacked RabbitMQ delivery was.
- Fan-out: each Herald reads `outcomes` under a consumer name per shard, and starts at the end of the topic the first time, as a freshly bound queue did.

## Alternatives rejected
- Keep RabbitMQ and add persistence: fixes the loss, keeps the weight.
- A dead-letter topic for retries: a broker-side mechanism for a client-side need, and one more topic whose order is not the order retries come due.
- A topic per routing key: Herald would need to know Genesis's bindings, and a new action type would need a new topic.

## Consequences
- Gains: messages survive a broker restart; the broker uses about 90 MiB; the bus is Rust end to end.
- Costs: Iggy uses io_uring, so its pod runs with an `Unconfined` seccomp profile. The SDK is pre-1.0 and pinned exactly, and moving it means moving the server image with it. The root password is read on the broker's first start only.
- What becomes harder: a data plane installed with RabbitMQ cannot be upgraded in place without draining it first: Herald acks an action with the control plane once it is published, so one still sitting in RabbitMQ when the broker is replaced is not leased again. More than one Genesis replica would need a consumer group, not a named consumer.
