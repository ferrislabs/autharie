# Identity space

Goal
  A deployment's console is split into Infrastructure (running it) and Identity (what its users live with), under one address, so IAM features have a place to grow and a customer's own staff can later be given Identity alone.

Context
  The console has one row of deployment tabs. Branding, Webhooks and Agents are in it next to Backups and Settings, and the realm is a `?realm=` search parameter read only by branding. `/platform` is the installation-wide section for Autharie's staff and is not touched. ADR 0014. Design validated on a mock (header with an Infrastructure | Identity switch, horizontal tabs per space, realm crumb).

Glossary
  Infrastructure: the deployment's existing space: overview, observability, backups, security, settings. Rejected: "Platform" (taken by the staff section).
  Identity: the space of what a deployment's users see and use: realms, applications, users, branding, agents, webhooks. Rejected: "Access".
  Realm crumb: the header selector of the realm being looked at.

Invariants
  - The realm of a page is the `$realm` path parameter and nothing else; no page reads `?realm=`.  [type: route param]
  - Changing realm keeps the section: from `.../realms/a/branding/email-provider` to realm b lands on `.../realms/b/branding/email-provider`.  [test: `swapRealm`]
  - Every address that worked before still works: it redirects, never 404s.  [test]
  - Tab visibility rules are unchanged: Agents needs a FerrisKey deployment and MANAGE_INSTANCES, Webhooks a FerrisKey deployment, Backups VIEW_BACKUPS.  [test]
  - A tab with no page behind it is not a link.  [type: `Tab.soon` carries no `to` use]
  - `/platform` and its tabs are unchanged.  [test: existing]

Out of scope
  - The realm overview and the realms list (PR 2).
  - Applications, Users, Organisations, Roles pages: their tabs show "Soon".
  - Any backend change, delegated access, the command palette.
  - Filtering webhooks or agents by realm.

Design
  Routes (D = /organisations/$organisationId/deployments/$deploymentId)
    D/identity                          → redirect to realms/<remembered or master>
    D/identity/realms/$realm            → redirect to branding/universal-login
    D/identity/realms/$realm/branding   → DeploymentBrandingLayout: universal-login, universal-login/$themeId, email-provider
    D/identity/agents                   → PageAgentsFeature
    D/identity/webhooks[/new|/$id|/$id/edit] → webhook pages
    D/{branding*,webhooks*,agents,customization} → redirects (realm from `?realm=`)
  Layouts: `IdentityLayout` is a sibling of `DeploymentLayout` (same reason as the deployment shell: nested, two headers). Both put a `SpaceSwitch` in the header.
  Tabs (pure functions, tested): infrastructure = Overview, Observability, Backups, Security, Settings. identity = Overview(soon), Applications(soon), Users(soon), Organisations(soon), Roles(soon), Branding, Agents, Webhooks.
  Realm: `useIdentityRealm()` = path param, else the realm last visited in this deployment (sessionStorage), else `master`. The crumb lists `GET .../branding/realms`.

Acceptance
  1. `D/identity/realms/acme/branding/universal-login` resolves under IdentityLayout and DeploymentBrandingLayout, not DeploymentLayout, with `realm = acme`.
  2. Universal login list, theme builder and email provider resolve under the same shells with `realm`.
  3. `D/identity/agents`, `D/identity/webhooks`, `/new`, `/$id`, `/$id/edit` resolve under IdentityLayout.
  4. Infrastructure tabs hold neither Branding, Webhooks nor Agents.
  5. Identity tabs: five "Soon" tabs are not links; Agents and Webhooks follow the visibility rules above.
  6. `D/branding`, `D/branding/universal-login?realm=acme`, `D/branding/email-provider`, `D/webhooks`, `D/webhooks/new`, `D/webhooks/$id`, `D/webhooks/$id/edit`, `D/agents`, `D/customization` and `D/settings/branding` redirect to their Identity address; the realm is kept.
  7. Bare `D/identity` and `D/identity/realms/$realm` redirect.
  8. The space switch is in both headers, with the open space marked.
  9. `swapRealm` keeps the section and the rest of the path.
  10. The realm crumb is shown on realm pages only, not on Agents and Webhooks.

Workstreams
  PR 1 shell (this one). PR 2 realms list and overview. PR 3 ADR: IAM data channel. PR 4 applications, users, organisations, roles. PR 5 ADR: delegated access. PR 6 command palette.

Risks / open questions
  - The IAM data channel (PR 3) decides what Applications and Users can show. Owner: PR 3's ADR.
  - Delegated access (PR 5) changes who reaches Identity at all. Owner: PR 5's ADR.

Verify (exit)
  cd apps/console && corepack pnpm exec tsc -b && corepack pnpm lint && corepack pnpm test
