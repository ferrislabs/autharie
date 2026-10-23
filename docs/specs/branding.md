# Branding on a deployment

Goal
  A customer shapes what their end users see from the Autharie console, the way Auth0's Branding tab does: the login pages (Universal login), the emails FerrisKey sends (templates and SMTP provider), with Custom domains and the phone entries announced as coming soon.

Context
  FerrisKey keeps, per realm: portal themes (design tokens, several themes, one active; since 0.8.0), one SMTP configuration (since 0.4.0) and email templates (since 0.5.0). Today the console has a "Customization" tab: seven colours and a radius, `master` realm only, through `deployments.iam_settings` and the legacy `/portal/theme` route.
  Facts about FerrisKey that shape this spec:
  - With no email template set for a type, FerrisKey sends no email (the security event `EmailNotSent / TEMPLATE_NOT_CONFIGURED` is recorded). There is no built-in default and no "send a test email" route.
  - An email template is `{ name, email_type, structure }`. The server renders the MJML from `structure`; the browser only sends JSON. A realm points at one template per type through `reset_password_template_id`, `magic_link_template_id`, `email_verification_template_id` in its settings.
  - The SMTP password is write-only: FerrisKey never returns it.
  - Portal themes have no server-side range validation; activating one checks the blocks each page needs; the active theme cannot be deleted.
  ADR 0011 (the data plane path, same as ADR 0007).

Glossary
  Branding: the console tab holding everything below. Replaces "Customization".
  Theme: one FerrisKey portal theme (design tokens) of one realm of one deployment. Rejected: "skin".
  Managed theme / template: a FerrisKey object whose name starts with `autharie:` followed by the Autharie id. The operator only touches managed ones.
  Email provider: the SMTP configuration of a realm. Rejected: "mailer".
  Email template: the template of one email type for one realm (one per type: Auth0's model).
  Starter template: the structure Autharie ships for each email type, offered as the starting point since FerrisKey has no default.

Invariants
  - The operator never creates, changes or deletes a FerrisKey theme or template that is not managed.  [test]
  - The SMTP password never appears in an IdentityInstance, a log line, an API response or the control plane database in clear. It is sealed with the existing envelope encryption, travels in the action payload once, is dropped from the stored payload when the action is published or fails, and reaches the operator in a Secret.  [test + review]
  - The password is write-only: saving the provider without a password keeps the stored one.  [test]
  - Each slice (themes, provider, templates) sends its whole desired state with a `revision` that only grows; Genesis ignores an older revision than the one applied.  [test]
  - A realm always has at most one active theme, and the active theme cannot be deleted.  [type + test]
  - There is one template per (realm, email type).  [DB unique + test]
  - A realm with no template for a type is shown as "No email is sent", never as working.  [test]
  - Colours are `#rrggbb`, fonts and sizes are bounded by the ranges below, a theme is at most 32 KiB, a template structure at most 256 KiB, a realm holds at most 10 themes.  [type: `ThemeTokens::parse`, `TemplateStructure::parse`; test]
  - Viewing needs VIEW_INSTANCES; changing needs MANAGE_INSTANCES. Themes keep the plan gate of today's branding (`IamFeature::Branding`); the provider and the templates have none.  [test]
  - The new slices are FerrisKey only; the console shows an empty state for another kind.  [test]

Out of scope
  - Page and layout editing of themes (FerrisKey's page builder).
  - Custom domains, Phone templates, Phone provider: shown with a "Soon" badge, nothing behind them.
  - A "send test email" button (FerrisKey has no route; the operator cannot prove delivery, only that FerrisKey accepted the configuration).
  - Localised templates.
  - Keycloak deployments.

Design
  Console (apps/console)
    - The "Customization" tab becomes "Branding" (`/branding`; `/customization` redirects). Under it a side navigation, shown in this order:
      Universal login · Custom domains (Soon) · Email templates · Email provider · Phone templates (Soon) · Phone provider (Soon).
      A "Soon" entry is not a link, is announced as unavailable, and carries a `Soon` badge. Needs a `Soon` variant of the status badge and a sub-navigation component (none exists).
    - A realm picker in the Branding header (realms come from `status.realms`, as for webhooks; default `master`). Every page below follows it.
    - Universal login: list of the realm's themes (name, active badge, state Pending/Applied/Failed), create / duplicate / rename / delete / activate / import / export JSON; a theme builder: panels for Colors, Typography, Buttons, Inputs, Widget, Page (spacing), a live preview of the sign-in card, Save and Save and activate.
    - Email provider: host, port, encryption (tls | starttls | none), username, password (write-only, "set" indicator), from name, from email; state; remove.
    - Email templates: three cards (reset password, magic link, email verification) with state ("No email is sent" when empty); each opens a visual block editor (MJML blocks: drag and drop, rich text, the type's variables listed), preview, Save, Reset to nothing. "Start from the starter template" when empty. The editor is FerrisKey's `builder-core` + `builder-mjml` (Apache-2.0), copied with attribution and adapted to this app's UI kit.
  Control plane (libs/autharie-domain, -core, -postgres, -api)
    - Tables `deployment_portal_themes`, `deployment_smtp_configs`, `deployment_email_templates`, `deployment_branding_sync` (revision and applied state per slice and realm). Migration also turns an existing `deployments.iam_settings.branding` into an active theme named "Custom" on `master`, then the legacy field is no longer written.
    - Each change records one action for its slice, with the realm's whole desired set and a new revision.
    - Ingests the three outcome kinds; per-object phase and message.
  Data plane
    - Genesis: three handlers; writes `spec.portalThemes`, `spec.smtp`, `spec.emailTemplates` on the IdentityInstance, the password into Secret `<instance>-smtp`; status watcher publishes the outcomes.
    - Operator: reconciles each against FerrisKey with the admin credentials it already holds. Themes: create or update managed themes, activate the chosen one, delete managed ones no longer wanted. Provider: PUT or DELETE `/realms/{realm}/smtp-config`. Templates: create or update the managed template, point the realm settings at it, or unset the pointer and delete it. When `spec.portalThemes` is present for `master`, the operator stops applying the legacy `spec.iam.branding`.
    - Herald: nothing.

Acceptance (Gherkin)
  @spec-br-1  Scenario: the Branding tab lists Universal login, Email templates and Email provider, and shows Custom domains, Phone templates and Phone provider as Soon
  @spec-br-2  Scenario: a Soon entry cannot be opened
  @spec-br-3  Scenario: a manager saves an SMTP provider and it becomes Applied; the password is never returned
  @spec-br-4  Scenario: saving the provider again without a password keeps the stored password
  @spec-br-5  Scenario: the SMTP password is absent from the stored action once it is published
  @spec-br-6  Scenario: a manager creates a theme, activates it, and the realm's login page uses it
  @spec-br-7  Scenario: the active theme cannot be deleted (409); a realm cannot hold an 11th theme (409)
  @spec-br-8  Scenario: an out-of-range colour or radius is refused (400)
  @spec-br-9  Scenario: a manager saves a reset-password template and the realm sends it; with none, the card says no email is sent
  @spec-br-10 Scenario: unmanaged themes and templates are left alone
  @spec-br-11 Scenario: an older revision redelivered after a newer one changes nothing
  @spec-br-12 Scenario: a viewer sees everything but cannot change anything (403)
  @spec-br-13 Scenario: a Free deployment without the branding feature sees themes locked and can still set the provider and templates
  @spec-br-14 Scenario: a deployment with the legacy branding gets an active "Custom" theme with the same colours

Simulation
  No. Ordering is handled by revisions, tested directly.

Workstreams (one PR each, in this order)
  S0 Docs: this spec and ADR 0011.
  S1 Console shell: Branding tab, side navigation, Soon badge, current page moved under Universal login. Email templates and Email provider are Soon here and open in S4 and S2. The realm picker lands with S2, the first slice that needs it. Owns apps/console. No backend.
  S2 Email provider: control plane, Genesis, CRD, operator, console page. 
  S3 Universal login: control plane, Genesis, CRD, operator, legacy migration, console list and theme builder.
  S4 Email templates: control plane, Genesis, CRD, operator, console cards and block editor (copied builder libs).
  Each of S2–S4 is vertical and splits into: CRD+operator, Genesis, control plane, console. Contracts below are frozen before any of them starts.

Frozen contracts
  // Action deployment.smtp v1
  { "deployment_id", "namespace", "revision": u64, "realm": str,
    "smtp": null | { "host", "port": u16, "encryption": "tls"|"starttls"|"none", "username", "from_email", "from_name", "password": str|null } }
  // password null = keep what is applied. Dropped from the stored payload once published.

  // Action deployment.portal_themes v1
  { "deployment_id", "namespace", "revision", "realm": str, "active_id": uuid|null,
    "themes": [ { "id": uuid, "name": str, "config": PortalThemeConfig } ] }
  // PortalThemeConfig is FerrisKey's camelCase shape (colors, fonts, borders, spacing).

  // Action deployment.email_templates v1
  { "deployment_id", "namespace", "revision", "realm": str,
    "templates": [ { "id": uuid, "email_type": "reset_password"|"magic_link"|"email_verification", "name": str, "structure": json } ] }

  // IdentityInstance
  spec.smtp: [ { realm, host, port, encryption, username, fromEmail, fromName } ]  + spec.smtpRevision: u64      // password only in Secret <instance>-smtp, key "<realm>"
  spec.portalThemes: [ { realm, activeId?, themes: [ { id, name, config } ] } ]  + spec.portalThemesRevision: u64
  spec.emailTemplates: [ { realm, templates: [ { id, emailType, name, structure } ] } ]  + spec.emailTemplatesRevision: u64
  status.smtp: [ { realm, phase: Pending|Applied|Failed, message? } ]
  status.portalThemes: [ { realm, themes: [ { id, ferriskeyId?, phase, message? } ] } ]
  status.emailTemplates: [ { realm, templates: [ { id, ferriskeyId?, phase, message? } ] } ]

  // Outcome (Genesis → control plane), kinds "smtp", "portal_themes", "email_templates"
  { "deployment_id", "kind", "revision", "realm", "items": [ { "id"?, "ferriskey_id"?, "phase", "message"? } ] }

  // Token ranges (ThemeTokens::parse)
  colours #rrggbb · fonts.baseSize 10–24 · weights 100–900 in steps of 100 · sizePct 50–300 · radii and border weights 0–64 · spacing 0–128 · widgetShadow none|small|large

  // API (organisation member routes, realm in the path)
  GET/PUT/DELETE /organisations/{o}/deployments/{d}/branding/{realm}/smtp
  GET/POST       /organisations/{o}/deployments/{d}/branding/{realm}/themes
  PUT/DELETE     /organisations/{o}/deployments/{d}/branding/{realm}/themes/{t}
  POST           /organisations/{o}/deployments/{d}/branding/{realm}/themes/{t}/activate
  GET/PUT/DELETE /organisations/{o}/deployments/{d}/branding/{realm}/email-templates/{email_type}
  GET            /organisations/{o}/deployments/{d}/branding/realms           → [str]

Risks / open questions
  - Assumption to confirm: the provider and the templates are not behind a plan gate; themes keep today's.
  - Assumption to confirm: one template per email type and realm (Auth0's model), though FerrisKey can hold several.
  - Whether `PUT /realms/{realm}/settings` is a partial update. If it replaces other settings, the operator must read, merge and write. Owner: S4 operator, check first.
  - Instances older than FerrisKey 0.8.0 have no multi-theme API, older than 0.5.0 no templates: the slice reports Failed with "FerrisKey too old".
  - A theme or template renamed by hand in FerrisKey becomes unmanaged and is recreated. Accepted, documented.
  - The copied block editor brings @dnd-kit, tiptap and mjml-browser into the console bundle; load it lazily on the template route.
  - Legacy migration: while `spec.portalThemes` is not yet applied, the legacy branding must keep working. The operator switches over only when the new field is present.

Verify (exit)
  cargo clippy --workspace --all-targets -- -D warnings
  cargo nextest run --workspace (with DATABASE_URL, REQUIRE_DATABASE_URL=1)
  pnpm --dir apps/console lint && pnpm --dir apps/console test && pnpm --dir apps/console build
  helm lint charts/autharie-dataplane
  End to end on k3d: @spec-br-3, -6, -9 by hand with a mail catcher.
