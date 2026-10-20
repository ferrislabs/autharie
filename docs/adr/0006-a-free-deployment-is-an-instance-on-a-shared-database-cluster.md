# 0006. A free deployment is a FerrisKey instance on a shared database cluster

Status: proposed
Date: 2026-10-20

## Context
A free deployment was a realm on a FerrisKey shared by many tenants (a cell). The tenants of a cell share its version, its configuration and its failures, and a tenant cannot be upgraded, moved or configured on its own. A paid deployment is a FerrisKey of its own with a CloudNativePG cluster of its own. Of the two, the database cluster is what costs too much to give away: one Postgres per tenant, with its storage and its backups. Everything else in a data plane is already shared: Herald, the Iggy broker, the operator, the Gateway.

## Decision
A free deployment is a FerrisKey instance like a paid one, with one replica and small resources. Its database is one database and one owner role on a single CNPG cluster per data plane, shared by every free deployment placed on it. The operator creates the role and the database over SQL as superuser, idempotently, and drops both when the deployment is deleted. Each shared database is dumped daily with `pg_dump` to the object store. Paid offers keep a cluster of their own. Cells and realms are removed.

## Alternatives rejected
- Keep realms on a shared FerrisKey: cheapest per tenant, but a tenant's lifecycle is bound to every other tenant's, which is what a free user trying the product would notice first.
- A CNPG cluster per free deployment: the isolation of a paid deployment, at the cost of a paid deployment.
- Roles declared in the shared `Cluster`'s `spec.managed.roles`: one object written by every free deployment's reconcile, where a stale write drops another tenant's role.
- CNPG `Database` resources for the databases: they cannot create the role and its password, so SQL is needed anyway, and two mechanisms would own one tenant.

## Consequences
- Gains: a free tenant is an instance that can be upgraded, configured and later moved to a paid offer on its own; a free deployment costs a FerrisKey pod and a database, not a cluster.
- Costs: the shared cluster is one point of failure for every free tenant of a data plane; the operator holds superuser credentials to it; the number of free tenants per data plane is bounded by Postgres connections.
- What becomes harder: restoring one tenant is a dump restore, not a CNPG point-in-time recovery. Moving a free deployment to a paid offer needs a database copy from the shared cluster to a cluster of its own.
