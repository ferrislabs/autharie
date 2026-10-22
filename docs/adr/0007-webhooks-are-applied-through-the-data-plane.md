# 0007. A deployment's webhooks are applied through the data plane

Status: proposed
Date: 2026-10-22

## Context
FerrisKey manages webhooks per realm through its admin API, generates their signing secret itself, and keeps a record of every delivery. Customers need to manage them from the Autharie console. The control plane does not hold an instance's admin credentials and cannot reach an instance on a private network or in a customer's cloud; only the data plane can.

## Decision
The control plane stores the desired webhooks of a deployment and sends them whole in an action. Genesis writes them into the IdentityInstance (header values into a Secret), the operator reconciles them against FerrisKey, touching only webhooks named `autharie:<id>`, and reports their state. A secret FerrisKey generates goes back to the control plane in an outcome and is stored there encrypted. Herald reads deliveries with a view-only FerrisKey client and indexes them with the organisation's logs.

## Alternatives rejected
- The control plane calls each instance's admin API over its public hostname: synchronous and simpler, but the control plane would hold admin credentials of every instance, and an instance that is private or in a customer cloud would have no webhooks.
- Leave webhooks to FerrisKey's own admin console: works today, but the customer leaves the product for a core integration task, and nothing in Autharie knows they exist.

## Consequences
- Gains: works for every distribution; the control plane never holds instance credentials; the same path as branding.
- Costs: changes apply in seconds, not instantly; a secret crosses the in-cluster bus once per rotation; five more fields on the IdentityInstance resource.
- What becomes harder: a managed webhook renamed by hand in FerrisKey is no longer recognised and is recreated.
