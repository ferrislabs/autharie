# A free deployment is a FerrisKey instance of its own

Goal
  A free deployment runs its own FerrisKey (API and webapp) whose database lives on a Postgres cluster the data plane shares between free deployments.

Context
  Until now a free deployment was a realm on a shared FerrisKey called a cell (`libs/autharie-domain/src/cells/`, `realm.*` actions in Genesis). A tenant then shares a FerrisKey version, its configuration and its failures with every other tenant of the cell, and cannot be moved, upgraded or given its own settings. A paid deployment is already a FerrisKey of its own, with a CloudNativePG cluster of its own. What makes that too costly to give away is the database: a CNPG cluster per tenant. The rest of a data plane is already shared: Herald, the Iggy broker, the operator, the Gateway.
  Decision: ADR 0006. No free tenant exists in production, so nothing is migrated.

Glossary
  Shared database cluster: the one CNPG cluster of a data plane that holds the database of every free deployment placed on it. Rejected: "pool", "cell".
  Shared database: one database and its owner role on the shared database cluster, belonging to exactly one free deployment.
  Dump: a `pg_dump` of one shared database, written to the object store. Rejected: "backup" (the word for a CNPG base backup of a whole cluster).

Invariants
  - Only `Offer::Free` uses a shared database; every other offer keeps a managed cluster of its own.  [type: `DatabasePlacement` is decided from the offer in one function, `Offer::database()`]
  - A shared database is reachable only with its own role: the role owns exactly one database and `CONNECT` is revoked from `PUBLIC`.  [test: integration, against a real Postgres]
  - Two free deployments never share a database or a role. The names derive from the deployment id, not from its display name.  [type: `SharedDatabaseName::for_deployment`]
  - Deleting a free deployment drops its database and its role, and nothing else on the cluster.  [test]
  - A free role holds at most 10 connections (`CONNECTION LIMIT`), so one tenant cannot starve the shared cluster.  [test: integration]
  - An organisation on `Plan::Free` holds at most one deployment.  [test: existing `Plan::Free.max_instances() == 1`]
  - A free deployment's FerrisKey runs one replica of each workload, with the resources of `Offer::Free`.  [type: `Offer::resources`, `Topology::SINGLE`]
  - The superuser credentials of the shared database cluster never leave the data plane's system namespace.  [test: rendered chart + operator unit test]

Out of scope
  - Restoring a dump from the console. This change writes dumps and documents a manual restore; a console restore is a follow-up.
  - Moving a free deployment to a paid offer (database from the shared cluster to a cluster of its own).
  - Putting paid offers on the shared database cluster.
  - Idle free deployments being stopped or reclaimed.
  - More than one shared database cluster per data plane.

Design
  Domain (control plane)
    - `Offer::Free` stops being pooled: it is placed like any other shared-mode offer, on a shared data plane with room.
    - `Offer::database() -> DatabasePlacement { Managed, Shared }`. Free is `Shared`, every other offer `Managed`.
    - `Offer::Free.resources()` becomes a real size (100m CPU, 256 MiB, 1 GiB of database storage accounted on the shared cluster).
    - The `deployment.create` payload carries `database: "shared" | "managed"` (payload version stays 1, absent reads as `managed`).
    - Removed: `Distribution::Pooled`, cells, realms, tenant administrators, the cells settings, `/platform/cells*`, and the `realm.*` actions.
  CRD (`IdentityInstance`)
    - `DatabaseMode` gains `Shared { cluster: SharedClusterRef }`. `managedCluster` stays the default.
  Operator
    - For `Shared`: ensures the role and the database on the shared cluster (SQL as superuser, idempotent), writes the instance's database Secret in the tenant namespace (host, port, database, user, password, uri), then runs the same migration Job, Deployments and HTTPRoute as a managed instance.
    - On delete: `DROP DATABASE ... WITH (FORCE)`, then `DROP ROLE`.
    - Dumps: a CronJob per shared instance in its namespace, daily, `pg_dump -Fc` to `<bucket>/dumps/<deployment-id>/<timestamp>.dump`, 7 kept.
  Data plane chart
    - `sharedDatabase.enabled` (default true): one CNPG `Cluster` in the release namespace with superuser access enabled, `instances: 1`, its own PVC size. The operator learns its name and namespace from env.
  Genesis
    - Maps the payload's `database` to `DatabaseMode`. Drops the realm handlers.
  Errors
    - Operator: `SharedDatabaseError { Unreachable, Refused(String) }`, surfaced as the instance's `Ready=False` reason.

Acceptance (Gherkin)
  @spec-free-1  Scenario: an organisation on the free plan creates a free deployment
    Given an organisation on Plan::Free with no deployment
    When it creates a deployment with offer free
    Then the deployment is placed on a shared data plane
    And Genesis receives deployment.create with database "shared" and the free resources

  @spec-free-2  Scenario: a free deployment's FerrisKey uses its own database on the shared cluster
    Given an IdentityInstance with database mode shared
    When the operator reconciles it
    Then a role and a database named after the deployment id exist on the shared cluster
    And the tenant namespace holds a Secret pointing at that database
    And no CNPG Cluster exists in the tenant namespace

  @spec-free-3  Scenario: one shared database cannot be read with another's role
    Given two free deployments A and B
    Then connecting to A's database with B's role is refused

  @spec-free-4  Scenario: deleting a free deployment drops only its database
    Given two free deployments A and B
    When A is deleted
    Then A's database and role are gone
    And B's database still answers

  @spec-free-5  Scenario: a second free deployment is refused
    Given an organisation on Plan::Free with one deployment
    When it creates another
    Then it is refused with the instance limit error

  @spec-free-6  Scenario: a paid deployment keeps a cluster of its own
    When an organisation on Plan::Starter creates a deployment with offer standard
    Then Genesis receives database "managed"

  @spec-free-7  Scenario: a shared database is dumped daily and kept for a week
    Given a free deployment
    Then its namespace holds a daily dump CronJob writing under dumps/<deployment-id>/
    And at most 7 dumps are kept

  @spec-free-8  Scenario: end to end on the local cluster
    Given the local stack with a data plane on k3d
    When a free deployment is created from the console
    Then its FerrisKey answers at https://<name>.<zone> and an administrator can sign in

Simulation
  No. The concurrency that matters (two reconciles of one instance) is covered by idempotent SQL (`IF NOT EXISTS`, `ON CONFLICT`) and tested by running the reconcile twice.

Workstreams
  W1 Shared database cluster in the data plane chart. Owns charts/autharie-dataplane (sharedDatabase values and template, operator env). Depends on nothing.
  W2 CRD and operator, shared mode. Owns libs/autharie-crds (DatabaseMode), libs/autharie-operator-core (shared database reconcile, delete, Secret). Depends on W1 for the env contract only.
  W3 Dumps. Owns the CronJob builder in the operator. Depends on W2.
  W4 Control plane and Genesis: Offer::Free placed as an instance with database shared. Owns libs/autharie-domain (offers, deployments), libs/autharie-core, libs/genesis-core (payload mapping), console offer copy. Depends on W2's payload contract.
  W5 Removal of cells and realms. Owns every file listed under cells/realms, plus a down-migration-safe migration dropping their tables. Depends on W4.
  Order: W1 and W2 together, then W3 and W4, then W5. One PR per workstream.

Frozen contracts
  // libs/autharie-domain/src/offers/mod.rs
  pub enum DatabasePlacement { Managed, Shared }
  impl Offer { pub fn database(self) -> DatabasePlacement }

  // deployment.create payload v1, new optional field
  "database": "shared" | "managed"   // absent = "managed"

  // libs/autharie-crds, IdentityInstance spec.database
  mode: managedCluster | shared
  shared: { clusterName: String, clusterNamespace: String }

  // Operator env, set by the data plane chart
  AUTHARIE_SHARED_DATABASE_CLUSTER, AUTHARIE_SHARED_DATABASE_NAMESPACE

  // Names on the shared cluster, from the deployment id (uuid, dashes removed)
  database: free_<32 hex>   role: free_<32 hex>

Risks / open questions
  - Network policies: a tenant namespace must reach the shared cluster's `-rw` Service in the system namespace. Owner: W1, checked on k3d.
  - Connection count: FerrisKey's pool is fixed in its code at 5 to 100 connections. Each free role is capped at 10; with max_connections 300 a data plane carries 28 free deployments at their cap. A burst past 10 fails that tenant's requests. Follow-up: make FerrisKey's pool size configurable (FerrisKey repository). Owner: W1 (values), W2 (role cap).
  - One shared cluster is one point of failure for every free tenant of a data plane. Accepted for the free offer; dumps are the recovery.

Verify (exit)
  cargo clippy --workspace --all-targets -- -D warnings
  cargo nextest run --workspace
  make test-integration
  helm lint charts/autharie-dataplane
  pnpm --dir apps/console lint && pnpm --dir apps/console test
  The @spec-free-8 scenario by hand on k3d.

Amended: 2026-10-20 FerrisKey's pool is 5-100, not configurable; free roles get CONNECTION LIMIT 10.
