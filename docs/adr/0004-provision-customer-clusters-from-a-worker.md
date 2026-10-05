# 0004. Provision customer clusters from a lease-based worker over data plane rows

Status: proposed
Date: 2026-10-04

## Context
A Kapsule cluster takes up to twenty minutes to become ready, and a customer cluster is built in an account we do not control, so it can fail on quota, region or credential. Creating the deployment must not hold an HTTP request open for that long. The create call already records a `DataPlane` (`Customer`, `Provisioning`) and a `Pending` deployment in one transaction. Several control plane replicas run at once, a replica can crash mid-build, and `provision` creates resources without checking the inventory first, so it is not safe to run twice over a half-built cluster.

## Decision
A worker in the control plane loops over `data_planes` rows. It claims customer planes still in `Provisioning` with no Herald binding, using `FOR UPDATE SKIP LOCKED` and a `provisioning_claimed_until` lease, so two replicas never build the same plane and a crashed replica's claim expires and is retried. Before every build it calls `deprovision`, which is idempotent and driven by the inventory, so leftovers of an earlier attempt are released first. The result is applied with `UPDATE ... WHERE status = 'provisioning'`, so a plane failed or disabled meanwhile is never overwritten. A provider error ends as `Failed` with the typed `ProvisionError` text as reason. A second pass releases the clusters of deleted deployments and of `Failed` planes, revokes the Herald identity, and marks the plane `Disabled`; a failed release is logged and retried on the next pass.

## Alternatives rejected
- Synchronous provisioning in the request: holds a connection and a transaction for minutes, fails the whole create on a client timeout while the cluster keeps building, and leaves nothing to resume after a crash.
- An action `dataplane.provision` claimed from the actions table: that table models work the data plane pulls from the control plane, and the cluster that would claim it does not exist yet. It also needs a second claim and retry mechanism whose state would have to be reconciled with the data plane's own status.

## Consequences
- Gains: the create call stays fast, a crash is recovered by lease expiry, and the status and failure reason a customer sees live on the data plane itself.
- Costs: the lease must exceed the longest build (`--customer-cloud-claim-lease-seconds`, default 45 minutes), so a crashed build is retried only after that; a replica holds one claim per plane for its whole build.
- What becomes harder: a build that outlives its lease is started twice. A second provider reuses the worker as is, but the lease then has to be per provider.
