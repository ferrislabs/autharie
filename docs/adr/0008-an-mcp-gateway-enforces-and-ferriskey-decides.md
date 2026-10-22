# 0008. An MCP gateway enforces, FerrisKey decides

Status: proposed
Date: 2026-10-22

## Context
Agents reach Autharie's API and customers' FerrisKey through MCP. Each call must be authorized per tool, with rules the customer controls. MCP 2026-07-28 makes the server an OAuth resource server that validates its token's audience and never passes a client's token through, and it mirrors the method and tool name in headers for intermediaries. FerrisKey already evaluates Cedar policies behind an AuthZEN 1.0 endpoint and supports RFC 8693 token exchange with `act` delegation.

## Decision
One stateless gateway, `apps/mcp-gateway`, sits in front of every MCP server Autharie exposes. It validates the caller's token for its own canonical URI, asks the realm that owns the resource for a decision through AuthZEN (one evaluation per tool call, one batch per tool list), exchanges the caller's token for one addressed to the tool server with the agent in `act`, forwards, and records the decision. FerrisKey is the PDP; the gateway holds no policy.

## Alternatives rejected
- Authorization inside each MCP server: every server reimplements the same checks, and a customer's rules would live in code.
- An embedded policy engine in the gateway (OPA, Cedar locally): a second source of truth beside the IAM the customer bought, and no shared decision log.
- Forwarding the caller's token: forbidden by MCP, and it erases which agent acted.

## Consequences
- Gains: one place for discovery, audience checks, decisions and audit; the same rules language for every server; tool servers stay simple.
- Costs: a PDP round trip per call (batch for lists); the PDP being down denies everything.
- What becomes harder: a tool server that needs the caller's original token cannot have it; it gets an exchanged one.
