# 0011. A deployment's login themes, SMTP provider and email templates are applied per realm through the data plane

Status: proposed
Date: 2026-10-23

## Context
FerrisKey keeps portal themes, one SMTP configuration and email templates per realm, behind its admin API. Branding today covers only the `master` realm, with seven colours and a radius, written through the legacy `/portal/theme` route. Customers need the login pages and the emails of their own realms, and the SMTP password is a secret the control plane must forward without ever showing it again.

## Decision
Themes, the SMTP provider and email templates are three separate slices, each scoped by deployment and realm. The control plane stores the desired state and sends it whole in an action with a revision. Genesis writes it into the IdentityInstance (the SMTP password into a Secret), the operator reconciles it against FerrisKey and reports what it applied. Themes and templates are named `autharie:<id>` in FerrisKey and the operator touches nothing else; the SMTP configuration, which FerrisKey holds once per realm, is the one object Autharie owns outright. An outcome kind per slice brings the state back.

## Alternatives rejected
- One `branding` document holding all three: a theme change would resend the SMTP password, and a failure in one slice would block the other two.
- The control plane calls the instance's admin API: it would hold admin credentials of every instance and could not reach a private one (same reason as ADR 0007).
- Keep branding on `master` only: a customer's end users sign in on their own realms.

## Consequences
- Gains: the same path as webhooks, so it works for every distribution; the three slices ship and fail independently.
- Costs: changes apply in seconds; three more groups of fields on the IdentityInstance; the legacy `spec.iam.branding` has to be migrated into a theme.
- What becomes harder: a managed theme or template renamed by hand in FerrisKey is no longer recognised and is recreated; a hand-made SMTP configuration is overwritten once the customer saves one in Autharie.
