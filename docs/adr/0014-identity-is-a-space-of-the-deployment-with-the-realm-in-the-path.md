# 0014. Identity is a space of the deployment, with the realm in the path

Status: proposed
Date: 2026-10-24

## Context
The console gives a deployment one row of tabs. Infrastructure (observability, backups, security, settings) and what the deployment's users live with (branding, webhooks, agents) sit side by side, and every new IAM feature (applications, users, organisations, roles) would add a tab to a bar that is already full. The realm, the scope of most of these features, is a `?realm=` search parameter that only the branding pages read.

`/platform` is something else: the installation-wide section for Autharie's own staff. It stays as it is.

## Decision
A deployment has two spaces under the same address: Infrastructure (what exists today, minus the three tabs below) and Identity, at `/organisations/$org/deployments/$dep/identity`. Branding, Webhooks and Agents move to Identity. The header gets a switch between the two spaces.

The realm is a path segment, `identity/realms/$realm/...`, for what belongs to a realm (branding now; applications, users, organisations and roles later). Webhooks and Agents stay deployment-wide for now and live at `identity/webhooks` and `identity/agents`.

The old addresses redirect: `branding`, `webhooks`, `agents` and `customization` under the deployment, with `?realm=` carried into the path.

## Alternatives rejected
- `/o/:org/d/:dep` short addresses: they would rewrite every link and every route test for a gain that can come later as an alias.
- Keeping `?realm=`: a realm that is part of what the page is cannot be shared or reloaded reliably as a filter, and it does not scale to the pages that depend on it.
- Identity as a fourth tab: it keeps the single bar that is already full.

## Consequences
- Gains: Infrastructure and Identity answer different questions and grow separately; the realm is in every address.
- Costs: old links need redirects for as long as they may be bookmarked; the branding pages lose their in-page realm picker for a crumb in the header.
- What becomes harder: a realm that is not in the path (webhooks, agents) needs the last realm remembered for the tabs that do need one.
