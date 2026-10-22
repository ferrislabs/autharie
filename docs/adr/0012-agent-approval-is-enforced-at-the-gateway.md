# 0012. Agent approval is enforced at the gateway

Status: proposed
Date: 2026-10-23

## Context
A customer must be able to say "no new agent calls anything until I approve it" and "this agent is blocked now". FerrisKey enables a client as soon as it registers through DCR or CIMD, has no pending state, and emits no event at registration. Every MCP call to a deployment already goes through the data plane's gateway, which knows the caller's client id.

## Decision
Approval and blocking are per-deployment lists the control plane owns and the operator renders into the gateway's configuration. The gateway refuses a blocked agent, and in approval mode an agent not approved, after validating its token and before asking the PDP, and records the refusal with a reason. Blocking also disables the FerrisKey client so it gets no new token.

## Alternatives rejected
- A pending state in FerrisKey: the right place in the long run, but it needs a FerrisKey change and release before anything works, and CIMD clients are recreated on fetch.
- Generated Cedar forbid policies: mixes access administration with the customer's own rules and depends on a context attribute every PDP call must carry.

## Consequences
- Gains: immediate effect, including on tokens already issued; pending agents surface through the refusals they cause; no FerrisKey change.
- Costs: the lists travel through the data plane like policies; a call is refused only at the gateway, so a token still works on non-MCP FerrisKey endpoints until the client is disabled.
- What becomes harder: moving approval into FerrisKey later means migrating these lists.
