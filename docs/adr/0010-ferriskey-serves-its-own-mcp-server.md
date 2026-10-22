# 0010. FerrisKey serves its own MCP server

Status: proposed
Date: 2026-10-22

## Context
Customers' agents need to manage their IAM (users, clients, roles, organisations, webhooks). The admin API is FerrisKey's, with its own permission model.

## Decision
The MCP server for a FerrisKey instance is a FerrisKey crate (`ferriskey-api-mcp`) mounted under the instance's root path at `/mcp`, calling FerrisKey's services directly and checking FerrisKey's permissions on every tool. Autharie's data plane gateway fronts it for policy decisions and token exchange.

## Alternatives rejected
- An Autharie-side adapter calling FerrisKey's admin API: a second copy of FerrisKey's model to keep in step with every release, and an extra hop with admin credentials.
- One shared MCP server for all instances: it would need credentials to every instance.

## Consequences
- Gains: tools follow FerrisKey's API as it evolves; a self-hosted FerrisKey gets an MCP server too.
- Costs: the feature ships with FerrisKey releases; Autharie needs a FerrisKey version that has it.
- What becomes harder: changing a tool means a FerrisKey release.
