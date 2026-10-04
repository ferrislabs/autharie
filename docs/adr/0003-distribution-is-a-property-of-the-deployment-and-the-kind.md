# 0003. Distribution is declared per deployment kind

Status: proposed
Date: 2026-10-04

## Context
A deployment is placed on a shared data plane or on a dedicated one. Self-hosting through helm and a customer cloud cluster are new options. The customer cloud mode costs us support and test effort on infrastructure we cannot see, so it should cover only what we have validated: the FerrisKey stack.

## Decision
`Distribution` is a type with three variants: `Shared`, `SelfHosted`, `CustomerCloud`. Each `DeploymentKind` declares which distributions it accepts. `Ferriskey` accepts all three. `Keycloak` accepts `Shared` and `SelfHosted`. The request type is built through a constructor that rejects an unsupported pair, so an invalid combination cannot reach placement.

## Alternatives rejected
- A boolean `byoc` on the deployment: cannot carry the credential and profile, and invites a state with a profile and no credential.
- Validation in the API handler only: another entry point (console, Android, CRD) could skip it.

## Consequences
- Gains: the supported matrix is one function, testable, and visible in the type.
- Costs: adding a kind or a distribution means editing that function.
- What becomes harder: nothing notable. Allowing `CustomerCloud` for another kind is one line plus its validation work.
