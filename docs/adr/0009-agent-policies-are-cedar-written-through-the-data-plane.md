# 0009. Agent policies are Cedar, written by the console through the data plane

Status: proposed
Date: 2026-10-22

## Context
Customers need to say which agent may call which tool, for whom and when, without learning a policy language, and the rules must be enforced by their own FerrisKey. The control plane cannot reach an instance; webhooks already established the desired-state path through the data plane.

## Decision
The console's policy builder edits a small rule model (effect, who, what, when) that compiles to Cedar; raw Cedar is allowed for advanced rules. Policies are validated and simulated in the browser with cedar-wasm against the realm's schema, stored as desired state in the control plane, sent whole in an action, written into the IdentityInstance, and applied by the operator through FerrisKey's authorization management API. Only policies Autharie wrote are changed.

## Alternatives rejected
- A policy language of Autharie's own: a translation layer to maintain and a dialect nobody else speaks.
- Writing policies through the instance's public hostname: the control plane would need admin credentials of every instance and could not reach private or customer-cloud ones.
- Simulation through the data plane: a round trip of seconds for every keystroke.

## Consequences
- Gains: customers write rules in plain terms; FerrisKey enforces exactly what was shown; simulation is instant.
- Costs: the builder's model must stay a strict subset of Cedar; cedar-wasm adds weight to the console (lazy-loaded).
- What becomes harder: a policy edited by hand in FerrisKey is not reflected in the builder.
