# 0010. Autharie serves FerrisKey's MCP server, for now

Status: proposed
Date: 2026-10-22

## Context
Customers' agents need to manage their IAM (users, clients, roles, organisations, webhooks). The admin API is FerrisKey's, with its own permission model. Putting the MCP server inside FerrisKey ties it to FerrisKey's release cycle while the agent product is still taking shape in Autharie.

## Decision
For now the MCP server for a FerrisKey instance is an Autharie app, `apps/ferriskey-mcp`, one per data plane behind the data plane's gateway. It calls the instance's admin API over HTTP with the token the gateway exchanged, so FerrisKey's own permissions still apply on every call. The gateway's upstream URL names the instance; the server resolves that name to the instance's in-cluster API service and to nothing else. Once the tool set settles, it can move into FerrisKey as a crate mounted under `{root}/mcp`.

## Alternatives rejected
- A crate inside FerrisKey right away: every tool change waits for a FerrisKey release, and Autharie must roll out a new FerrisKey version to every deployment to ship it.
- One MCP server per deployment: a pod per instance for a feature most will not use.

## Consequences
- Gains: Autharie ships tools on its own cadence; no FerrisKey upgrade is needed to use them.
- Costs: the tools follow FerrisKey's admin API by hand and break if a route changes; an extra HTTP hop.
- What becomes harder: a self-hosted FerrisKey has no MCP server until it moves.
