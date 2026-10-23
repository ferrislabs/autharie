# 0011. Customer MCP servers are reached with exchanged tokens over public HTTPS

Status: proposed
Date: 2026-10-23

## Context
Customers want agents to call their own tools (business APIs, internal workflows) under the same rules as FerrisKey's. The data plane's MCP gateway can front several upstreams (ADR 0008). Two things had to be settled: how the gateway proves itself to a server Autharie does not run, and which addresses the data plane may call on a customer's behalf.

## Decision
The gateway reaches a customer server only with a FerrisKey token obtained by token exchange, whose audience is a client the operator creates for that server (`mcp-<slug>`) and which carries the agent in `act`. The customer server validates it with the realm's JWKS. Only `https://` URLs to public addresses are accepted; the data plane checks the resolved address, connects to that address, and follows no redirect, both in the gateway and in tool discovery.

## Alternatives rejected
- A static API key or header per server: a secret to store encrypted, rotate and push to every data plane, and the server learns nothing about the user or the agent.
- Private and in-cluster URLs: turns the gateway into a way into the data plane (SSRF); can be revisited for dedicated data planes with an explicit allow-list.

## Consequences
- Gains: no customer secret in Autharie; the customer server sees who asked and through which agent; one SSRF rule for every outbound call.
- Costs: the customer server must validate a JWT (audience, issuer, signature).
- What becomes harder: servers on private networks need another way in (a public endpoint or a later dedicated-plane option).
