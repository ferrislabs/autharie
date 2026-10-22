# Agents: MCP access to Autharie and FerrisKey, authorized by FerrisKey

Goal
  An AI agent reaches Autharie's API and a customer's FerrisKey through MCP, authenticated by the IdP that owns the resource, and every call it makes is allowed or denied by rules the customer writes in the console and FerrisKey evaluates.

Context
  Autharie sells an IAM. Agents (assistants, internal automations, coding agents) are a new kind of client: they act for a user or on their own, call tools rather than pages, and need finer rules than "has a role". The promise: the IAM a customer buys authenticates their agents and decides what each one may do, through open standards (OAuth 2.1, MCP 2026-07-28, OpenID AuthZEN 1.0).
  Facts this spec rests on:
  - MCP 2026-07-28: a server is an OAuth 2.1 resource server; it MUST publish Protected Resource Metadata (RFC 9728), MUST validate the token audience, MUST NOT pass a client's token through. Client ID Metadata Documents (CIMD) are the preferred registration; DCR is deprecated. Transport is stateless POST; every request carries `Mcp-Method`, and `tools/call` carries `Mcp-Name` (the tool), mirrored from the body for intermediaries. `tools/list` MAY vary with the caller's authorization. Enterprise-Managed Authorization (ID-JAG) is a stable extension; client credentials is a draft extension.
  - OpenID AuthZEN Authorization API 1.0 is final: `POST /access/v1/evaluation`, optional `POST /access/v1/evaluations` (batch), search endpoints, `/.well-known/authzen-configuration`.
  - FerrisKey (origin/main 90f53e89): Cedar 4.13 engine with a per-realm schema and static/template-linked policies; `POST /api/realms/{realm}/authzen/access/v1/evaluation` (caller: a service account with `EvaluateAuthorization`), enabled by `AUTHZ_ENABLED`; RFC 8414/OIDC discovery with S256, CIMD (`cimd_enabled`), DCR (`dcr_enabled`), RFC 8707 `resource` → `aud` (realm `allowed_resources`), RFC 8693 token exchange with `act` delegation and per-client exchange policies, client credentials, introspection, revocation, device grant.
  - Autharie's API validates a token's signature and expiry but not its `iss` or `aud`, and enforces no scope.
  ADRs 0008, 0009, 0010.

Glossary
  Agent: an OAuth client that calls tools, acting for a user (delegated) or for itself (autonomous). In FerrisKey an agent is a client. Rejected: "bot", "AI user".
  MCP gateway: Autharie's policy enforcement point in front of MCP servers. Rejected: "proxy" (it decides and exchanges tokens; it does not just forward).
  PDP: the FerrisKey realm that decides, through AuthZEN.
  Agent policy: a Cedar policy in a realm's FerrisKey that allows or forbids agent actions. Written in the console's policy builder.
  Tool server: an MCP server behind the gateway (the Autharie MCP server, a deployment's FerrisKey MCP server).

Invariants
  - No token reaches a tool server unless it was issued for that tool server: the gateway validates the caller's token for its own canonical URI, then exchanges it (RFC 8693) for a token whose audience is the tool server, carrying the agent in `act`.  [test]
  - Default deny: a tool call is forwarded only if the PDP returned `decision: true`; a PDP error, timeout or missing policy denies.  [test]
  - The decision for `tools/call` uses the tool name from the body; a request whose `Mcp-Name` header disagrees with the body is refused (400, -32020) before any decision.  [test]
  - `tools/list` only shows tools the caller may call, and is never cached across callers.  [test]
  - Autharie's API refuses a token whose `iss` is not the configured issuer or whose `aud` does not include the API's audience.  [test]
  - A deployment's agent policies are written only through Autharie's desired state (as webhooks are); policies created by hand in FerrisKey are left alone.  [test]
  - Every decision the gateway acts on is recorded (subject, agent, tool, decision, policy ids, time).  [test]

Out of scope (this chantier)
  - Customers' own MCP servers behind the gateway (the gateway is built for it; registering arbitrary upstreams comes next).
  - ID-JAG / Enterprise-Managed Authorization and private_key_jwt (FerrisKey gaps, phase 2 — see the gap table).
  - AuthZEN search endpoints.
  - Rate limits per agent.

Design
  Components
    apps/mcp-gateway (Rust, Autharie repo): stateless HTTP service. Per upstream it has a canonical URI, a PDP (issuer + realm + evaluation endpoint + service account), a token exchange client, and a tool server URL. For each POST: validate the bearer (JWKS, `iss`, `aud` = canonical URI); on 401 send `WWW-Authenticate: Bearer resource_metadata="…", scope="…"`; serve `/.well-known/oauth-protected-resource[/path]`; read `Mcp-Method`/`Mcp-Name` and the body; build AuthZEN requests (mapping below); for `tools/list` forward, then filter the result with one batch evaluation; exchange the token; forward; record the decision.
    apps/autharie-mcp (Rust, rmcp, Autharie repo): MCP server whose tools are a curated set of Autharie API operations (list organisations, deployments, a deployment's status/metrics/logs/backups, create/upgrade/stop a deployment, webhooks...), each a thin call to the API with the exchanged token. Behind the gateway at `https://app.<domain>/mcp` (canonical URI), PDP = the `autharie` realm.
    apps/ferriskey-mcp (Rust, rmcp, Autharie repo for now, see ADR 0010): tools over a FerrisKey instance's admin API (users, clients, roles, groups, organisations, webhooks, security events), called over HTTP with the exchanged token so FerrisKey's own permissions apply. One per data plane, behind the data plane's gateway at `https://<deployment host>/mcp`, PDP = that deployment's realm. The gateway's upstream URL names the instance (`/instances/<namespace>/<name>/mcp`); the server maps it to the instance's in-cluster API service and never to anything else.
    Organisation switch `mcp_enabled` (as in Mestier): off by default, changed by members who can manage the organisation. The Autharie API recognises a delegated call by the `act` claim (only the gateway may exchange to the API audience); on such a call an organisation without the switch answers 403 and is left out of the organisation list, and a delegated token only reaches the routes autharie-mcp calls. A deployment's agents (gateway route, FerrisKey MCP) are applied only while its organisation has the switch on.
    Data plane: one mcp-gateway per data plane, routed by hostname for every FerrisKey deployment with agents enabled. The operator enables `AUTHZ_ENABLED`, creates the gateway's service account (EvaluateAuthorization) and token-exchange policy, adds the canonical URI to the realm's `allowed_resources`, enables CIMD, and reconciles agent policies (desired state from the control plane, like webhooks).
    Control plane: agent settings per deployment (enabled, realm), agent policies (desired state: builder model + compiled Cedar), action `deployment.agent_policies`, outcome ingestion (applied/failed per policy), decision log search (Herald ships the gateway's decision records to the log index, kind `agent_decision`).
    Console: deployment tab "Agents": connection card (MCP URL, how to connect a client), policy builder (visual rules ↔ Cedar, validated and simulated in the browser with cedar-wasm against the realm schema), decision log.
  AuthZEN mapping (gateway → PDP)
    tools/call: subject {type: "User", id: sub, properties: {roles, groups, organisation}} — or {type: "Client", id: client_id} for an autonomous agent; action {name: "call_tool"}; resource {type: "Tool", id: "<server>/<tool>", properties: {server, tool, annotations}}; context {agent: client_id, delegated: bool, act, arguments: <selected primitive arguments>, time}.
    tools/list filter: one evaluations batch, one item per tool, same subject/context.
    resources/read, prompts/get: action "read_resource" / "get_prompt", resource type "Resource" / "Prompt".
  Cedar (FerrisKey built-in schema extended): entity types FerrisKey::Agent (= client), FerrisKey::Tool {server, tool}, FerrisKey::McpServer; actions call_tool, read_resource, get_prompt; context {agent, delegated, arguments, time}.
  Policy builder model (control plane stores it, compiles to Cedar): rule = effect (permit|forbid) + who (any agent | agents [client ids] | delegated for users with role/group) + what (tools by server/name/annotation e.g. read-only) + when (conditions on arguments, weekday/hour window). Raw Cedar rules allowed for advanced users.

FerrisKey gap table (from the audit)
  | capability | status | to add | phase | effort |
  | Authz management HTTP API (schema, policies, templates, enable/disable) | service exists, no routes | routes + permissions | 1 | M |
  | Policy validate / simulate endpoint | missing | POST validate (+ dry-run evaluation with supplied entities) | 1 | S |
  | AuthZEN batch `/access/v1/evaluations` | trait default, not routed | route + semantics | 1 | S |
  | `/.well-known/authzen-configuration` at spec path | realm-suffixed variant | keep, document | 1 | S |
  | Cedar built-in types for agents/tools/MCP | missing | schema extension | 1 | S |
  | MCP server over admin API | missing | apps/ferriskey-mcp in Autharie for now (ADR 0010) | 1 | L |
  | Decision log (persist + query) | missing | table + route (sampling) | 2 | M |
  | PKCE without method defaults to `plain` | gap | refuse plain for public clients | 1 | S |
  | private_key_jwt client auth | missing | RFC 7523 client auth | 2 | M |
  | ID-JAG / jwt-bearer grant (RFC 7523) | missing | grant + token type | 2 | L |
  | AuthZEN search endpoints | missing | | 2 | M |
  | ReBAC tuples read by the engine | table only | | 3 | L |

Acceptance (Gherkin)
  @spec-ag-1  An agent connects to https://app.<domain>/mcp with no token, gets 401 with resource_metadata, discovers the `autharie` realm, signs the user in with PKCE and `resource`, and lists only the tools its policies allow.
  @spec-ag-2  A tools/call allowed by policy reaches autharie-mcp with an exchanged token (aud = Autharie API, act = agent) and returns the API's data.
  @spec-ag-3  A tools/call with no permitting policy is denied (JSON-RPC error, 403 semantics) and the decision is recorded.
  @spec-ag-4  A token issued for another resource is refused by the gateway; a token with the wrong iss/aud is refused by Autharie's API.
  @spec-ag-5  A mismatched Mcp-Name header is refused before evaluation.
  @spec-ag-6  A customer enables agents on a FerrisKey deployment; https://<host>/mcp serves FerrisKey's tools through the data plane gateway; the deployment realm decides.
  @spec-ag-7  A policy built in the console (e.g. "agent X may call read-only tools for users in group support, weekdays 8-18") is validated in the browser, applied to the deployment's FerrisKey, and changes the next decision.
  @spec-ag-8  Decisions appear in the console's decision log with the policy that decided.
  @spec-ag-9  The PDP being unreachable denies every call.

Simulation
  No. Concurrency is per request and stateless.

Workstreams
  Wave 0 (now): A0 security — Autharie API validates iss/aud (and parses scopes for later). Autharie repo.
  Wave 1 (parallel):
    F1 FerrisKey: authz management routes + validate/simulate + batch evaluations + Cedar agent types + PKCE plain refusal. FerrisKey repo.
    F2 Autharie: apps/ferriskey-mcp (rmcp) over a FerrisKey instance's admin API.
    A6 Autharie: organisation switch `mcp_enabled` (migration, API, console setting) and delegated-call enforcement in the API (403, list filter, route allowlist).
    A1 Autharie: apps/mcp-gateway (PEP) + libs it needs, with a fake PDP and fake upstream in tests. Starts from Mestier's `apps/mcp-gateway` (Mestier ADR 0004: same PEP, AuthZEN and exchange design, built to serve other ferrislabs products), extended with several upstreams, the batch filter of `tools/list`, the `Mcp-Name` check and decision records.
    A2 Autharie: apps/autharie-mcp (rmcp) over the API.
  Wave 2 (parallel, after the contracts are proven):
    A3 Data plane: operator/genesis/herald plumbing (enable authz, gateway service account, allowed_resources, CIMD, agent policies desired state, decision records shipping), data plane chart (gateway deployment + routes).
    A4 Control plane: agent settings, agent policies (model + compile to Cedar), action/outcome, decision search, control plane chart (gateway + autharie-mcp + routes), realm terraform (allowed_resources, CIMD, token exchange policy).
    A5 Console: Agents tab, policy builder with cedar-wasm, decision log.
  Wave 3: end to end on k3d with a real MCP client (rmcp client example) for @spec-ag-1..9.

Frozen contracts
  // Gateway upstream config (TOML/env), one per upstream
  { canonical_uri, issuer, jwks_uri?, pdp: { evaluation_url, evaluations_url, client_id, client_secret }, exchange: { token_url, client_id, client_secret, audience }, upstream_url, server_name, scopes_supported[] }
  // Decision record (gateway → stdout JSON line, shipped by Herald/collector)
  { kind: "agent_decision", time, server, method, tool?, subject_type, subject_id, agent, delegated, decision, policies[], reason?, request_id }
  // FerrisKey authz management (new routes, under {root}/realms/{realm}/authz)
  GET/PUT schema; GET/POST policies; GET/PUT/DELETE policies/{id}; POST policies/validate; POST simulate {policies?, entities, request}; GET/POST templates…; GET/PUT state
  // IdentityInstance (agents)
  spec.agents: { enabled, realm, policies: [ { id, name, cedar } ], policiesRevision }   status.agents: { phase, gatewayClientReady, policies: [ { id, phase, message? } ] }

Risks / open questions
  - The FerrisKey MCP server makes FerrisKey's admin surface agent-reachable; FerrisKey's permissions apply through the exchanged token and the PDP decides first. It must map an instance name only to that instance's API service (no URL from the request). Owner: F2.
  - Policy builder ↔ Cedar round-trip: only builder-made policies are editable visually; raw Cedar is shown read-only in the builder and editable as text. Owner: A5.
  - cedar-wasm bundle size in the console; lazy-loaded on the Agents tab. Owner: A5.
  - Phase-2 gaps (ID-JAG, private_key_jwt) limit workforce agents to delegated (user sign-in) and client_secret client credentials for now.

Verify (exit)
  Autharie: cargo clippy -D warnings, cargo nextest --workspace (DB), console lint/test/build, helm lint both charts.
  FerrisKey: its own CI commands (cargo, front), DCO-signed commits.
  End to end on k3d: @spec-ag-1..9.
