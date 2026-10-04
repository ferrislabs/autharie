# Customer cloud provider

Goal
  A customer registers a cloud provider credential and runs a FerrisKey deployment on a cluster created in their own account, sized by a mode.

Context
  Today a deployment runs on a shared data plane operated by us, or on a data plane the customer installs with helm. Epic #21 settled that a dedicated deployment is a cluster of its own, created by a push path (`ClusterProvisioner`, action `dataplane.provision`) and that the cluster pulls its own work afterwards. Issue #29 plans a Scaleway adapter whose credentials live in control-plane configuration. This spec adds the case where the credential belongs to the customer and the bill goes to their account, so a customer can try the product at the cost of their own infrastructure and pay us only for paid control-plane features.
  Related: ADR 0001 (customer credentials), ADR 0002 (provisioning engine), ADR 0003 (distribution modes).

Glossary
  - Cloud credential: a service account key a customer registers for one provider. Belongs to an organisation. Never leaves the control plane in clear.
  - Distribution: how a deployment is hosted. `Shared`, `SelfHosted` (customer installs the data plane with helm), `CustomerCloud` (we create the cluster in the customer account).
  - Cluster mode: a fixed preset (`dev`, `standard`, `ha`) that sets defaults and limits for a customer cloud cluster.
  - Cluster profile: the mode plus the customer's adjustments inside its limits (node type, node range).

Invariants
  - A `CustomerCloud` deployment has exactly one cluster, created for it and deleted with it.  [type: `Distribution::CustomerCloud` carries one `CloudCredentialId` and one `ClusterProfile`]
  - Only a `DeploymentKind::Ferriskey` deployment can use `CustomerCloud`.  [type: `Distribution::allowed_for(kind)`, checked in the constructor of the request]
  - A cluster profile is valid for its mode: node range, replication and node count satisfy the table below.  [type: `ClusterProfile::new` returns `Result`]
  - A credential is never returned by the API, written to a log, put in an action payload, or stored unencrypted.  [test: no secret in log capture, actions table and API responses]
  - A credential in use cannot be deleted.  [test]
  - Deleting the deployment deprovisions the cluster; deprovision is idempotent and reports what it removed.  [test against the adapter, plus an inventory check]
  - The kubeconfig of a created cluster is used once to bootstrap and then discarded.  [test]
  - The control plane never needs inbound access to the customer cluster after bootstrap.  [design, pull model]

Out of scope
  - Providers other than Scaleway. The port is shaped for a second one, nothing more.
  - Billing, plans and the paid control-plane features. Only the cost estimate shown before creation is in scope.
  - Custom profiles beyond the adjustments below, and spot or reserved instances.
  - Kubernetes upgrades on the customer cluster (open question 3).
  - Domains and hostnames for provisioned deployments (open question 4).
  - Other deployment kinds (Keycloak).

Design
  Domain:
    - `CloudCredential { id, organisation_id, provider, label, scope_check, created_at }`: no secret in the entity. The secret is held by the credential store port.
    - `Provider` enum: `Scaleway`.
    - `ClusterMode` enum: `Dev | Standard | Ha`, with `limits()` returning the table below.
    - `ClusterProfile { mode, control_plane, node_type, min_nodes, max_nodes, replication }`, built only through `ClusterProfile::new(mode, control_plane, node_type, min_nodes, max_nodes, replication, catalog) -> Result<_, ProfileError>`. `control_plane` is an offer id from the provider catalog (on Scaleway: mutualized or a dedicated size), not an enum we maintain.
    - `Distribution` enum: `Shared | SelfHosted | CustomerCloud { credential_id, profile }`.
    - `DataPlaneAllocation` gains `Customer { organisation_id, deployment_id, credential_id }`.
    - Use cases: `RegisterCloudCredential`, `DeleteCloudCredential`, `EstimateClusterCost`, `CreateDeploymentOnCustomerCloud`, `ResizeCluster` (mode change inside the profile rules).
  Ports:
    - `CloudCredentialStore` (generic): `put`, `get_for_provisioning`, `delete`. Adapter wraps the secret with the transit key provider that backups already use. Not `dyn`.
    - `ClusterProvisioner` (existing, generic): `provision` takes the resolved credential and the profile, `deprovision` takes the data plane id and resolves its credential through the store, `resize` is added.
    - `ProviderCatalog` (generic): `offers(provider, credential, region)` returns control plane offers and node types with prices, used to validate a profile and to estimate cost.
    - `CredentialVerifier` (generic): checks that a credential has the permissions we require and no more than needed.
  Adapters:
    - Credential store: OpenBao transit wrapping, ciphertext in PostgreSQL.
    - Provisioner and catalog: Scaleway (Kapsule), extends issue #29.
    - Verifier: Scaleway IAM policy inspection.
  Errors:
    - `ProfileError`: `NodeRangeInvalid`, `BelowModeFloor`, `ReplicationExceedsNodes`, `NodeTypeUnavailable`, `ControlPlaneUnavailable`, `ControlPlaneNotAllowedForMode`.
    - `CredentialError`: `Invalid`, `MissingPermissions { missing }`, `ExcessPermissions { extra }`, `InUse`.
    - `ProvisionError`: `QuotaExceeded`, `RegionUnavailable`, `NodePoolNeverConverged`, `CredentialRejected`. Each is a readable `Failed` reason on the data plane.

  Cluster modes (limits):

  | Mode | min nodes | FerrisKey replicas | Database | Autoscaling | Control plane |
  |---|---|---|---|---|---|
  | dev | 1, max 1 | 1 | 1 instance | no | mutualized only |
  | standard | 2 or more | 2 | 1 instance | yes, up to the profile max | mutualized or dedicated |
  | ha | 3 or more | 2 | 3 instances | yes, up to the profile max | mutualized or dedicated |

  Decided: `standard` floors at 2 nodes, `ha` at 3, to keep the entry price low.

  Rules: `min_nodes <= max_nodes`; `min_nodes` is at least the mode floor; replication never exceeds `min_nodes`; the node type and the control plane offer must appear in the provider catalog for the region; `dev` accepts the mutualized control plane only, because a dedicated one costs more than the rest of a `dev` cluster. A mode change `dev -> standard -> ha` goes through `ResizeCluster` and keeps the data plane. The reverse is refused when it would drop below the replicas in use.

Acceptance (Gherkin)
  @spec-ccp-1  Scenario: a customer registers a credential with the required permissions
  @spec-ccp-2  Scenario: a credential with missing or excess permissions is refused with the list
  @spec-ccp-3  Scenario: a credential is never returned by the API, logged, or put in an action payload
  @spec-ccp-4  Scenario: a credential used by a deployment cannot be deleted
  @spec-ccp-5  Scenario: a `dev` profile with two nodes is refused
  @spec-ccp-6  Scenario: a `ha` profile with two minimum nodes is refused
  @spec-ccp-7  Scenario: a node type absent from the region catalog is refused
  @spec-ccp-8  Scenario: the estimated monthly cost is shown for a valid profile before creation
  @spec-ccp-9  Scenario: a non-FerrisKey deployment cannot use `CustomerCloud`
  @spec-ccp-10 Scenario: a `CustomerCloud` deployment creates one cluster, bootstraps it, registers the data plane and reaches `Active`
  @spec-ccp-11 Scenario: a quota failure ends as a `Failed` data plane with a readable reason
  @spec-ccp-12 Scenario: the kubeconfig is discarded after bootstrap
  @spec-ccp-13 Scenario: deleting the deployment deletes the cluster and the inventory shows nothing left
  @spec-ccp-14 Scenario: deprovision called twice succeeds and removes nothing the second time
  @spec-ccp-15 Scenario: resizing `dev` to `standard` keeps the data plane and adds a node and a replica
  @spec-ccp-16 Scenario: resizing `ha` to `dev` is refused while replicas exceed the target
  @spec-ccp-17 Scenario: a control plane offer absent from the region catalog is refused
  @spec-ccp-18 Scenario: a `dev` profile with a dedicated control plane is refused
  @spec-ccp-19 Scenario: a `ha` profile with min 3 and max 10 nodes of a catalog instance type is accepted

Simulation
  No. The provisioning steps are sequential and idempotent, and the failure shapes are covered by adapter tests against a recorded Scaleway API. Revisit if teardown after a mid-creation crash proves flaky: that is the one place a deterministic simulation would earn its cost.

Workstreams
  - W1 domain: `Provider`, `ClusterMode`, `ClusterProfile`, `Distribution`, errors, allocation variant, validation. Depends on nothing. Owns `libs/autharie-domain/src/dataplane/` and `libs/autharie-domain/src/deployments/`.
  - W2 credentials: `CloudCredentialStore` port, transit adapter, migration, API routes, `CredentialVerifier`. Depends on W1. Owns `libs/autharie-api/src/handlers/cloud_credentials/`, `libs/autharie-postgres/`, `libs/autharie-transit/`.
  - W3 provisioning: extend `ClusterProvisioner` with credential, profile and `resize`; Scaleway adapter and catalog (issue #29); teardown with inventory. Depends on W1 and W2, and on the bootstrap chart of epic #21. Owns `libs/autharie-core/src/infrastructure/provisioner/`.
  - W4 console: credential page, mode and profile form, cost estimate, status with failure reason. Depends on W2 and W3 API shape. Owns `apps/console/src/domain/cloud-providers/`.
  - W5 e2e: scenarios and properties for the acceptance lines. Depends on all. Owns `libs/autharie-e2e/`.

Frozen contracts
  ```rust
  pub enum Provider { Scaleway }
  pub enum ClusterMode { Dev, Standard, Ha }
  pub struct ClusterProfile { /* private fields */ }
  impl ClusterProfile {
      pub fn new(mode: ClusterMode, control_plane: ControlPlaneOffer, node_type: NodeType,
                 min_nodes: u8, max_nodes: u8, replication: Replication,
                 catalog: &ProviderOffers) -> Result<Self, ProfileError>;
  }
  pub enum Distribution {
      Shared,
      SelfHosted,
      CustomerCloud { credential_id: CloudCredentialId, profile: ClusterProfile },
  }
  pub trait CloudCredentialStore: Send + Sync {
      fn put(&self, organisation_id: OrganisationId, provider: Provider, secret: SecretString)
          -> impl Future<Output = Result<CloudCredentialId, CredentialError>> + Send;
      fn get_for_provisioning(&self, id: &CloudCredentialId)
          -> impl Future<Output = Result<SecretString, CredentialError>> + Send;
      fn delete(&self, id: &CloudCredentialId)
          -> impl Future<Output = Result<(), CredentialError>> + Send;
  }
  ```

Risks / open questions
  1. Lightest node that runs the full stack (FerrisKey, database, Herald, Genesis, operator, gateway, bus): unmeasured, and the `dev` price depends on it. Owner: Nathael. Measure before W3 fixes the `dev` node type.
  2. A `light` stack profile (single replica, no log search) for `dev` may be needed if the full stack does not fit. Owner: Nathael, depends on 1.
  3. Who upgrades Kubernetes and the data plane on a customer cluster. Owner: Nathael. Until decided, clusters are created on a supported version and upgrades are manual.
  4. Hostnames and domains for provisioned deployments: the payload carries no hostname today (epic #21). Owner: Nathael.
  5. Exact Scaleway permissions to require, and whether a project-scoped key is enough. Owner: Nathael, settled in W2 by reading the provider IAM documentation.
  6. Cost estimate accuracy: node prices only, or storage and egress too. Proposal: node prices and storage, with egress stated as not included.
  7. Control plane tier names and sizes are read from the catalog at runtime; no value is hardcoded. Owner: W3.

Verify (exit)
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  SQLX_OFFLINE=true cargo nextest run --workspace
  make test-integration
  pnpm --dir apps/console test
  pnpm --dir apps/console lint

Amended: 2026-10-04 control plane offer added to the profile, taken from the provider catalog, after the Scaleway options were described (mutualized or dedicated, instance type, autoscaling range).
Amended: 2026-10-04 `standard` floor fixed at 2 nodes.
Amended: 2026-10-04 provisioning is asynchronous: creating a `CustomerCloud` deployment records the data plane in `Provisioning` and the deployment in `Pending`, and a background worker builds the cluster. @spec-ccp-10 and @spec-ccp-11 are observed on the data plane's status and failure reason, not on the create call.
Amended: 2026-10-04 deleting the deployment is also observed on the data plane: the worker releases the cluster from the inventory, revokes the Herald identity and marks the data plane `Disabled` (@spec-ccp-13, @spec-ccp-14). A `Failed` data plane keeps its status and reason after its resources are released. See ADR 0004.
