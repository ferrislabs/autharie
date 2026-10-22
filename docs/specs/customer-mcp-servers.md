# Customer MCP servers behind the gateway

Goal
  A customer registers their own MCP server on a FerrisKey deployment; agents reach it through the deployment's MCP gateway, and every call is decided by the same agent policies, canvas and decision log as FerrisKey's own tools.

Context
  The agents feature (docs/specs/agent-authorization.md) puts the data plane's MCP gateway in front of one tool server per deployment: the FerrisKey MCP server. The gateway already takes a list of upstreams, each with its canonical URI, PDP and token exchange (apps/mcp-gateway/src/config.rs), and FerrisKey's Cedar schema names tools generically (`Tool` with `server` and `tool`). What is fixed: the operator only renders the FerrisKey upstream (libs/autharie-operator-core/src/domain/agents/service.rs, `upstreams`), and the console's tool catalogue is a hard-coded list of two servers (apps/console/src/domain/agents/tool-catalogue.ts). This spec opens the gateway to the customer's own tools: their business APIs and workflows, protected by the IAM they bought.
  Decisions taken with the owner (2026-10-23): the gateway reaches a customer server with an exchanged FerrisKey token, never a stored secret; only public HTTPS URLs are accepted; tools are discovered automatically. ADR 0011.

Glossary
  Customer server: an MCP server the customer runs and registers on a deployment. Rejected: "custom upstream", "external tool".
  Server slug: the customer server's short name, unique in its deployment; it prefixes its tools (`<slug>/<tool>`) and its path (`/mcp/<slug>`). Rejected: "server id" (the id is a UUID).
  Tool discovery: the data plane listing a customer server's tools (`tools/list`) and reporting them to the control plane.

Invariants
  - A customer server URL is `https://`, has no userinfo, and its host is a DNS name or a public IP; the control plane refuses anything else at registration.  [type: `CustomerServerUrl::try_from`]
  - The data plane never connects to a customer server whose resolved addresses include a private, loopback, link-local, CGNAT, multicast, unspecified or cloud-metadata address, and connects to the address it checked (no second resolution). Redirects are not followed. Applies to the gateway and to discovery.  [test]
  - A slug matches `^[a-z][a-z0-9-]{1,30}$` and is never `ferriskey` or `autharie`; unique per deployment.  [type + DB unique index]
  - A customer server never receives the agent's token: the gateway exchanges it (RFC 8693) for a token whose audience is that server's audience client, with the agent in `act`. Same rule as every other tool server.  [test]
  - Default deny holds for customer tools exactly as for FerrisKey tools; a tool missing from the last discovery is still decided by the PDP (the catalogue is a hint for the console, not an allow-list).  [test]
  - Discovery results are bounded: at most 200 tools per server, names `^[A-Za-z0-9_.-]{1,64}$` (others dropped and counted), descriptions cut to 512 characters, response body capped at 1 MiB, 10 s timeout.  [test]
  - At most 10 customer servers per deployment.  [test]
  - Removing a customer server removes its gateway upstream, its route and its audience client; its policies stay (they are the customer's) and show as referring to an unknown server.  [test]

Out of scope
  - Static secrets or API keys toward the customer server (ADR 0011 keeps them out).
  - Private or in-cluster URLs, including on dedicated data planes.
  - Resources and prompts discovery (`resources/list`, `prompts/list`); only tools.
  - Per-tool argument schemas in the canvas (names, descriptions and hints only).
  - The Autharie control plane gateway (app.<domain>/mcp); customer servers live on deployments only.

Design
  Domain (libs/autharie-domain/src/agents/servers.rs)
    CustomerServer { id: ServerId(Uuid), deployment_id, slug: ServerSlug, name, url: CustomerServerUrl, enabled, revision, discovery: Option<Discovery>, created_at, updated_at }
    Discovery { phase: Ready | Unreachable | Refused | Invalid, tools: Vec<DiscoveredTool>, dropped: u32, message?, discovered_at }
    DiscoveredTool { name, description?, read_only: bool, destructive: bool }  // from MCP tool annotations readOnlyHint / destructiveHint
    ServerSlug, CustomerServerUrl: newtypes with TryFrom carrying the invariants above.
    Use cases: register, update (name, url, enabled), remove, request discovery, record outcome. Every change bumps the agents revision and republishes `deployment.agents` (the action already carries the whole agents state).
  Ports: AgentServerRepository (postgres adapter). Static dispatch like the existing agents ports.
  Persistence: migration `agent_servers` (id, deployment_id FK, slug, name, url, enabled, discovery jsonb, discovery_requested_at, created_at, updated_at, unique(deployment_id, slug)).
  Action payload `deployment.agents` gains `servers: [{ id, slug, url, enabled, discovery_requested_at? }]`.
  CRD IdentityInstance: spec.agents.servers[{ id, slug, url, enabled, discoveryRequestedAt? }]; status.agents.servers[{ id, phase, tools[], dropped, message?, discoveredAt? }]. Outcome `deployment.agents` carries `servers` the same way; the control plane stores it.
  Operator (libs/autharie-operator-core, agents)
    - Per enabled customer server: ensure an audience client `mcp-<slug>` in the realm (like `ensure_audience_client` for `<realm>-realm`), a token-exchange policy from the gateway client to it, the canonical URI `https://<deployment host>/mcp/<slug>` in the realm's allowed_resources and resource-owners.
    - Render one gateway upstream per customer server: canonical_uri as above, same issuer/pdp as the FerrisKey upstream, exchange.audience = `mcp-<slug>`, upstream_url = the customer URL, server_name = slug.
    - HTTPRoute: `/mcp/<slug>` and `/.well-known/oauth-protected-resource/mcp/<slug>` on the deployment host → the gateway.
    - Discovery: on spec change, when `discoveryRequestedAt` is newer than the last discovery, and every 6 h: client-credentials token for the gateway client with audience `mcp-<slug>`, then MCP `initialize` + `tools/list` (follow `nextCursor` up to the 200-tool bound) through a guarded HTTP client (address check, pinned connect, no redirects, limits). Result goes to status.
  Guarded HTTP client: one module shared by the gateway and the operator (new small crate `libs/autharie-egress`), so the SSRF rule has a single implementation and one test suite.
  Gateway: no new concept. It already routes by canonical URI; it must use the guarded client for upstreams whose host is not in-cluster (flag on the upstream config: `egress = "public"`).
  Control plane API (under /organisations/{org}/deployments/{dep}/agents)
    GET    /servers                     → { data: [CustomerServer] }
    POST   /servers  { slug, name, url, enabled? }   → 201 { data }
    PUT    /servers/{server_id}  { name, url, enabled }
    DELETE /servers/{server_id}
    POST   /servers/{server_id}/discover               → 202
    Permissions as agent policies: VIEW_INSTANCES to read, MANAGE_INSTANCES to write; FerrisKey deployments only; refused to delegated tokens.
  Console
    - tool-catalogue becomes data: the static FerrisKey/Autharie entries plus the deployment's discovered servers (hook `useAgentServers`). Everything that reads the catalogue (canvas menus, tool picker, labels, templates) takes it as input.
    - Overview tab: "Your MCP servers" section: list (name, URL, status chip, tool count, last discovery, Refresh), Add/Edit dialog (slug derived from name, editable; URL validated client-side with the same rule), Remove with confirmation naming the policies that refer to it. Connection guide shows each server's own MCP URL `https://<host>/mcp/<slug>`.
    - Canvas: "Add what ▸ Whole server ▸" and the tool picker list customer servers; tools carry their read-only/destructive hints.

Acceptance (Gherkin)
  @spec-ms-1  Registering `http://…`, `https://10.0.0.5/mcp`, `https://user:pw@x.example/` or a slug `ferriskey` is refused with a field error; `https://tools.example.com/mcp` with slug `crm` is accepted and bumps the agents revision.
  @spec-ms-2  The operator, given a customer server in spec, creates the audience client `mcp-crm`, the exchange policy, the allowed resource, the HTTPRoute, and a gateway upstream with server_name `crm`.
  @spec-ms-3  Discovery against a fake MCP server returns its tools with hints into status; the control plane stores them; GET /servers shows them.
  @spec-ms-4  A host resolving to 127.0.0.1 or 169.254.169.254 is never connected to (gateway and discovery); a 302 to another host is not followed; status says Refused.
  @spec-ms-5  An agent calls `tools/call crm/list_invoices` through `https://<host>/mcp/crm`: the PDP decides on resource `crm/list_invoices`; when allowed, the fake server receives a token with aud `mcp-crm` and `act` = the agent, never the agent's token.
  @spec-ms-6  A canvas rule "support may call read-only tools on crm" compiles to Cedar on `resource.server == "crm"` and changes the next decision.
  @spec-ms-7  Removing the server removes upstream, route and audience client; its policies remain and the canvas marks the server unknown.
  @spec-ms-8  A server returning 500 tools, a 2 MiB body or a 30 s response yields a bounded or Unreachable discovery, never a crash.

Simulation
  No: per-request, stateless; discovery is a bounded request with a timeout.

Workstreams
  WS-1 Domain + persistence + API + action payload + outcome ingestion (control plane).
  WS-2 libs/autharie-egress guarded client (+ tests with a local resolver stub), used by gateway and operator.
  WS-3 Operator: audience client, exchange policy, allowed resources, route, upstream rendering, discovery, status. CRD fields.
  WS-4 Console: catalogue as data, servers section, canvas integration.
  Order: WS-2 and WS-1 first (contracts below are frozen), WS-3 and WS-4 after.

Frozen contracts
  CustomerServer (API) = { id, slug, name, url, enabled, mcp_url?, discovery?: { phase, tools: [{ name, description?, read_only, destructive }], dropped, message?, discovered_at }, created_at, updated_at }
  spec.agents.servers[] = { id, slug, url, enabled, discoveryRequestedAt? }
  status.agents.servers[] = { id, phase: "Ready"|"Unreachable"|"Refused"|"Invalid", tools[], dropped, message?, discoveredAt? }
  Gateway UpstreamConfig gains `egress: "cluster" | "public"` (default "cluster").

Risks / open questions
  - A customer server that cannot validate a FerrisKey JWT cannot be used (ADR 0011). Owner: product; revisit with demand.
  - CIMD-style metadata fetches and discovery share the SSRF rule; DNS rebinding is handled by connecting to the checked address. Owner: WS-2.
  - Many servers on a shared data plane grow the gateway config; bounded by 10 per deployment. Owner: WS-3.

Verify (exit)
  cargo clippy --workspace --all-targets -- -D warnings
  make test && make test-integration
  cd apps/console && pnpm lint && pnpm exec tsc -b --noEmit && pnpm vitest run
  helm lint charts/autharie-dataplane
  End to end on k3d: @spec-ms-2..7 with a small MCP server on a public test host or a resolver override documented in the test.
