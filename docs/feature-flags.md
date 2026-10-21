# Feature flags

A feature flag switches a capability on or off without a release. Operators change flags from the console, globally or for one organisation.

## What a flag is

- A variant of the `Feature` enum in `libs/autharie-domain/src/features/mod.rs`. The set of flags is defined in code. The console cannot create or delete one.
- Each flag has a stable string key (`Feature::as_str`), a human description, and an entry in `Feature::ALL`.
- The database stores only answers:
  - `feature_flags(key, enabled_by_default, updated_at, updated_by)`: the installation-wide default. A missing row means `false`.
  - `organisation_feature_flags(organisation_id, key, enabled, updated_at, updated_by)`: an override for one organisation. Deleted with the organisation.
- A key found in the database that no variant defines (a flag removed in a later release) is ignored when reading.

## Resolution order

For an organisation and a flag, the first that applies wins:

1. the organisation override, if one exists (`true` or `false`);
2. the installation default, if a row exists;
3. `false`.

This is `features::resolve`, a pure function with unit tests.

## Adding a flag

1. Add a variant to `Feature` in `libs/autharie-domain/src/features/mod.rs`.
2. Give it a key in `as_str` and a sentence in `description`, and add it to `Feature::ALL` (and update the array length).
3. Gate the behaviour in the application or domain layer, not in an HTTP handler: call `features::service::require_enabled(&repository, organisation_id, Feature::YourFeature)`. It returns `CoreError::FeatureNotEnabled`, which the API maps to `403` with code `E_FEATURE_NOT_ENABLED` and a message naming the feature.
4. Add the variant to the `Feature` type in the console (regenerate the client with `scripts/generate-api-client.sh`) and gate the screen with `useFeature(organisationId, key)` from `apps/console/src/domain/features/hooks/use-feature.ts`.
5. No migration is needed. Existing databases treat the new flag as off until someone sets it.
6. Tests that exercise the gated behaviour must enable the flag for their organisation in their setup, for example by inserting a row in `organisation_feature_flags`.

## Endpoints

Operator endpoints sit under `/platform` and are checked against platform rights, read from the database on every request.

| Method and path | Right | Purpose |
| --- | --- | --- |
| `GET /platform/features` | `view_estate` | Every flag with its key, description, `enabled_by_default`, and the number of organisation overrides on and off. |
| `PUT /platform/features/{key}` | `operate_fleet` | Body `{ "enabled_by_default": bool }`. Unknown key: `404`. Recorded in the fleet audit trail as `feature.default_changed` when the value changes. |
| `GET /platform/organisations/{organisation_id}/features` | `view_estate` | Every flag with `key`, `description`, `default`, `override` (`true`, `false` or `null`) and `effective`. Unknown organisation: `404`. |
| `PUT /platform/organisations/{organisation_id}/features/{key}` | `act_on_tenant` | Body `{ "enabled": bool \| null }`. `null` clears the override. The field is required. Unknown organisation or key: `404`. Recorded in the organisation audit trail as `organisation.feature.changed` when the value changes. |

Member endpoint:

| Method and path | Access | Purpose |
| --- | --- | --- |
| `GET /organisations/{organisation_id}/feature-flags` | Members who can view the organisation's deployments | `{ "enabled": ["customer_cloud"] }`: the effective keys only. |

The member endpoint is `feature-flags` rather than `features` because `GET /organisations/{organisation_id}/deployments/{deployment_id}/features` already lists the features a deployment's subscription opens.

## `customer_cloud`

- Key: `customer_cloud`.
- Description: deploy into the customer's own cloud account (bring your own cloud).
- Gate: `AutharieService::create_deployment` refuses a deployment whose distribution is `customer_cloud` with `403 E_FEATURE_NOT_ENABLED` unless the flag is enabled for the organisation. The caller must first hold the right to create deployments, so the flag state is not disclosed to non-members.
- Off by default. Enable it for one organisation with an override, or for everyone by switching the default on.
