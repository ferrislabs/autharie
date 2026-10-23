# Agent registry, approval and autonomous agents

Goal
  A deployment's owner sees every agent connected to their realm, can require that a new agent be approved before it calls anything, can block or remove an agent, and can create an autonomous agent (an n8n workflow, a bot) from the console and receive its credentials once.

Context
  The agents feature (docs/specs/agent-authorization.md) decides each call with Cedar policies, but the console cannot say which agents exist: nothing reads a realm's clients back to the control plane. FerrisKey facts (audit 2026-10-23): DCR and CIMD clients are enabled as soon as they register and are flagged `registration_source` Dynamic / MetadataDocument; there is no pending state, no event on registration, no `last_used`; clients can be listed, patched (`enabled`) and deleted through the admin API; client authentication is by secret only (no private_key_jwt). The gateway's decision records carry the agent's client id (`agent`), searchable for 30 days.
  Decisions taken with the owner (2026-10-22/23): approval is enforced at the gateway, not by a new FerrisKey state (ADR 0012); an autonomous agent's secret is shown once, sealed for the browser that asked (ADR 0013).

Glossary
  Agent registry: the per-deployment list of agents (FerrisKey clients that call tools), with their source, kind, access state and last activity. Rejected: "client list" (it holds agents only).
  Access state: approved, pending or blocked. Rejected: "status" (taken by deployment phases).
  Approval mode: a deployment setting; when on, only approved agents get past the gateway.
  Autonomous agent: an agent acting for itself with client credentials; created by Autharie. Delegated agent: an agent acting for a signed-in user (DCR or CIMD client).
  Sealed credential: a client secret encrypted by the data plane for a public key the browser generated for that one request.

Invariants
  - The operator only lists, disables, enables or deletes agent clients: clients with registration_source Dynamic or MetadataDocument, or created by Autharie as autonomous agents (client_id prefix `agent-`). It never touches any other client (mcp-gateway, console, admin-made clients).  [type: `AgentClientId` built only from such a client + test]
  - A blocked agent is refused by the gateway before any PDP call, whatever the policies, and its FerrisKey client is disabled.  [test]
  - In approval mode an agent not approved is refused before any PDP call; the refusal is recorded with reason `agent_pending` (blocked: `agent_blocked`), so a pending agent appears in the registry and the log.  [test]
  - Approval mode off keeps today's behaviour exactly (only blocked agents are refused).  [test]
  - A client secret never exists in clear outside the data plane and the requesting browser: the control plane stores only the sealed form, deletes it once delivered or after 15 minutes, and never logs it.  [test]
  - Sealing uses RSA-OAEP with SHA-256 to a public key of at least 2048 bits supplied with the request; any other key is refused.  [type: `SealingKey::try_from` + test]
  - The registry reported by the data plane is bounded (500 clients, newest first, `truncated` flag).  [test]

Out of scope
  - private_key_jwt or other secretless client auth (FerrisKey gap).
  - Revoking existing refresh tokens or consents per client (FerrisKey has no endpoint; blocking at the gateway covers MCP calls).
  - Notifications (email, webhook) when a new agent appears.
  - The registry on the control plane's own gateway (app.<domain>/mcp).

Design
  Domain (libs/autharie-domain/src/agents/registry.rs)
    AgentClient { client_id, name?, source: Dynamic | MetadataDocument | Autharie | Seen, kind: Delegated | Autonomous, enabled_in_ferriskey: Option<bool>, access: Approved | Pending | Blocked, created_at?, last_seen_at? }
    AgentAccess (stored per deployment): { approval_required: bool, approved: set<client_id>, blocked: set<client_id> }
    AutonomousAgent request: { request_id, name, description?, sealing_key: SealingKey } → desired creation; credential delivery { request_id, client_id, sealed, expires_at }.
    Access state is derived: blocked if in `blocked`; approved if in `approved` or approval mode off; else pending.
    An autonomous agent created through Autharie is added to `approved` when it is created: the owner asked for it.
    Use cases: set approval mode, approve, block, unblock, delete agent, create autonomous agent, rotate its secret, record outcome (registry + credentials), list registry (merges FerrisKey-reported clients, access sets, and last activity from decisions; clients only seen in decisions get source Seen).
  Persistence: migrations `agent_access` (deployment_id PK, approval_required, approved text[], blocked text[], deleted text[], clients_truncated), `agent_clients` (deployment_id, client_id, name, source, kind, enabled, created_at, reported_at, position; PK (deployment_id, client_id)), `agent_autonomous` (deployment_id, client_id, name, description, created_at), `agent_credentials` (request_id PK, deployment_id, client_id, kind, sealing_key, sealed, delivered_at, expires_at).
  Action `deployment.agents` gains `access: { approval_required, approved[], blocked[] }`, `autonomous: [{ request_id?, client_id, name, rotate_request_id?, sealing_key? }]`, `deleted_agents: [client_id]`.
  CRD IdentityInstance: spec.agents.access, spec.agents.autonomous[], spec.agents.deletedAgents[]; status.agents.clients[] { clientId, name?, source, kind, enabled, createdAt? }, status.agents.clientsTruncated, status.agents.credentials[] { requestId, clientId, sealed }. Outcome carries the same; the control plane moves credentials to `agent_credentials` and the registry to `agent_clients`.
  Operator
    - List clients (paginated `GET /realms/{r}/clients`), keep agent clients only (invariant), report newest 500.
    - Blocked agents: PATCH enabled=false; unblocked: enabled=true (only if Autharie disabled it).
    - Autonomous agents: create the client the control plane named `agent-<slug>-<6 random>` (confidential, service_account_enabled, client credentials only, no redirect), fetch its secret, seal it to the request's key, put it in status once; rotation deletes and recreates the client under the same id (FerrisKey cannot regenerate a secret). Deletion deletes the FerrisKey client.
    - Gateway config: every upstream of the deployment gets `access = { mode = "open"|"approval", approved = [...], blocked = [...] }`.
  Gateway (apps/mcp-gateway): after token validation and before the PDP, check the caller's client id against `access`; refuse with JSON-RPC error -32003 (403 semantics) and record the decision with `decision: false`, `reason: "agent_blocked" | "agent_pending"`, `policies: []`. `tools/list` for a refused agent returns the same error.
  Control plane API (under /organisations/{org}/deployments/{dep}/agents)
    GET    /clients                                  → { data: [AgentClient], truncated, approval_required }
    PUT    /access { approval_required }
    POST   /clients/{client_id}/approve | /block | /unblock
    DELETE /clients/{client_id}
    POST   /autonomous { name, description?, sealing_key (JWK, RSA) }        → 202 { data: { request_id } }
    POST   /autonomous/{client_id}/rotate { sealing_key }                    → 202 { data: { request_id } }
    GET    /credentials/{request_id}   → 200 { data: { client_id, sealed, token_url, audience } } once (then 410), 202 while pending, 404 after expiry
    Permissions: VIEW_INSTANCES to read the registry, MANAGE_INSTANCES for the rest; refused to delegated tokens; FerrisKey deployments only.
    Last activity: Quickwit terms aggregation on `agent` (max timestamp, 30 days) for the deployment; decisions search gains an `agent` filter.
  Console
    - New tab "Agents" (between Access map and Policies): registry table (name or client id, source badge, Delegated/Autonomous, access chip, last activity, actions Approve / Block / Unblock / Delete with confirmations), a "Pending" filter with a count badge on the tab, approval-mode switch with a one-line explanation.
    - "New autonomous agent" dialog: name + description → generates an RSA-OAEP-256 key pair with WebCrypto (non-extractable private key, memory only), POSTs the public JWK, polls the credential, decrypts, shows client id + secret once with copy buttons and ready-to-paste snippets (curl client_credentials with `resource`, n8n OAuth2 credential fields). Closing the dialog drops the key; "Rotate secret" repeats the flow.
    - Canvas: "Through which agent → Specific agents" and "Add who → Autonomous agent" pick from the registry by name (free text still allowed).
    - Activity tab: filter by agent; pending refusals are labelled "waiting for approval" with an Approve shortcut.

Acceptance (Gherkin)
  @spec-ar-1  A DCR client and a CIMD client registered in the realm appear in GET /clients with their source and kind; the mcp-gateway client and an admin-made client do not.
  @spec-ar-2  With approval mode on, a new agent's tools/list is refused with reason agent_pending; it shows as Pending in the registry; after Approve the next call reaches the PDP.
  @spec-ar-3  Blocking an approved agent refuses its next call with agent_blocked before the PDP, and its FerrisKey client is disabled; Unblock re-enables both.
  @spec-ar-4  Approval mode off: an unknown agent reaches the PDP as today.
  @spec-ar-5  Creating an autonomous agent returns a request id; the operator creates `agent-…`, seals its secret to the supplied key; GET /credentials returns it once, then 410; the stored sealed value is gone; the browser decrypts it and a client_credentials call with `resource` = the MCP URL gets a token the gateway accepts.
  @spec-ar-6  A sealing key under 2048 bits or not RSA is refused (400); a credential not fetched within 15 minutes is deleted.
  @spec-ar-7  Last activity in the registry matches the newest decision for that agent.
  @spec-ar-8  The operator never disables or deletes a client outside the agent set, even when asked (unit test with a forged desired state).

Simulation
  No: decisions are per request; the only shared state is configuration rendered by the operator.

Workstreams
  WS-1 Control plane: domain, migrations, API, action payload, outcome ingestion, credentials lifecycle, Quickwit aggregation.
  WS-2 Operator + CRD: client listing, enable/disable/delete, autonomous creation/rotation with sealing, gateway access rendering, status.
  WS-3 Gateway: access check before the PDP, decision reasons, config field.
  WS-4 Console: Agents tab, approval switch, autonomous dialog (WebCrypto), canvas pickers, activity filter.
  Order: freeze the contracts below; WS-1 and WS-3 in parallel, then WS-2, then WS-4.

Frozen contracts
  AgentClient (API) = { client_id, name?, source: "dynamic"|"metadata_document"|"autharie"|"seen", kind: "delegated"|"autonomous", enabled?: bool, access: "approved"|"pending"|"blocked", created_at?, last_seen_at? }
  Gateway UpstreamConfig gains `access: { mode: "open"|"approval", approved: [string], blocked: [string] }` (default open, empty).
  Decision record `reason` values added: "agent_blocked", "agent_pending".
  Sealed credential = base64url(RSA-OAEP-SHA256(secret utf-8)); sealing_key = public JWK { kty: "RSA", n, e, alg: "RSA-OAEP-256" }.

Risks / open questions
  - CIMD clients are re-upserted on each fetch; FerrisKey might re-enable a disabled one. The gateway refusal still holds, and the operator looks again every 30 s and disables a blocked client again. Owner: WS-2.
  - A registry of 500 is enough for a realm's agents; beyond, `truncated` tells the console. Owner: WS-1.
  - Pending agents only appear once they try a call (FerrisKey emits nothing at registration) or once the operator lists them. Owner: product.

Verify (exit)
  cargo clippy --workspace --all-targets -- -D warnings
  make test && make test-integration
  cd apps/console && pnpm lint && pnpm exec tsc -b --noEmit && pnpm vitest run
  End to end on k3d: @spec-ar-1..5.

Amended: 2026-10-23 FerrisKey has no client-secret regeneration route, so rotation deletes and recreates the client under the same client id.
Amended: 2026-10-23 a delivered credential keeps a secret-free tombstone row until it expires, so a second GET answers 410 and not 404.
Amended: 2026-10-23 the control plane names the autonomous client id when it takes the request (the operator creates that exact id), and keeps autonomous agents and deletions in `agent_autonomous` and `agent_access.deleted`.
Amended: 2026-10-23 GET /clients also carries `approval_required`, since the console switch needs the mode and the list is the one read.
Amended: 2026-10-23 an autonomous agent is approved at creation; found end to end, it showed as pending once approval mode was on.
