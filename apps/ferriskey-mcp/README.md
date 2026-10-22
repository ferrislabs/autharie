# ferriskey-mcp

An MCP server over a FerrisKey instance's admin API, for the deployments that run FerrisKey. It sits behind the data plane's MCP gateway (private, never exposed) and serves stateless streamable HTTP (MCP 2026-07-28) at:

```
/instances/{namespace}/{name}/mcp
```

`{namespace}` and `{name}` name the `IdentityInstance`. They are the only thing in the request that picks a target.

## How an instance is reached

The FerrisKey API base URL is built only from a configured template, never from the request or the token. `{namespace}` and `{name}` must each be a DNS-1123 label (lowercase letters, digits and `-`, 1 to 63 characters, alphanumeric at both ends). Anything else, a dot, a slash, an encoded slash, `..`, an uppercase letter, is answered 404 before any call is made. The template is checked at start-up: it must contain both placeholders and be an http or https URL with no credentials, query, fragment or dot segments.

Each request carries the token the gateway exchanged for it, whose audience is FerrisKey. Every tool forwards that token as `Authorization: Bearer ...` to the instance's admin API, which decides what the caller may do. The server has no credentials of its own, keeps no session and never stores a token. A request without a bearer is refused with 401. The API's refusals come back as tool errors (`isError: true`) carrying the status and message, not as protocol errors. Redirects are never followed.

## Configuration

| Flag | Variable | Default |
| --- | --- | --- |
| `--listen` | `FERRISKEY_MCP_LISTEN` | `0.0.0.0:8080` |
| `--api-url-template` | `FERRISKEY_MCP_API_URL_TEMPLATE` | `http://{name}-api.{namespace}.svc:3333` |
| `--root-path` | `FERRISKEY_MCP_ROOT_PATH` | `/api` |

The default template is the Service the operator creates for an `IdentityInstance` (`{name}-api`, port 3333), and the default root path is the `SERVER_ROOT_PATH` the operator gives the FerrisKey API. Routes are called under `{root}/realms/{realm}/...`.

`GET /healthz` answers 200 without a token.

## Realm

Every tool takes an optional `realm`. Without it the tool acts on the token's realm: the last segment of the `iss` claim when the issuer looks like `<base>/realms/<realm>`. The claim is only parsed, never verified here (the gateway and FerrisKey do that) and is used only to pick the realm part of a path; it can never change the host. A realm must be plain (`A-Z a-z 0-9 - _ .`, not made only of dots). When neither the argument nor the token gives a realm the tool fails with a tool error.

## Tools

Policies refer to a tool as `ferriskey/<tool>`. The names are stable. Routes are relative to `{root}/realms/{realm}`. Identifiers are validated as UUIDs before any call, so an argument cannot change the path it is placed in. Lists take `page` (from 1) and `limit` (1 to 100).

| Tool | API route | Read-only | Destructive |
| --- | --- | --- | --- |
| `list_realms` | `GET /users/@me/realms` | yes | |
| `list_users` | `GET /users` (`search`, `username`, `email`, `enabled`, `role_id`) | yes | |
| `get_user` | `GET /users/{u}` | yes | |
| `get_user_roles` | `GET /users/{u}/roles` | yes | |
| `list_roles` | `GET /roles` | yes | |
| `list_clients` | `GET /clients` (secrets removed from the output) | yes | |
| `get_client` | `GET /clients/{c}` (secrets removed from the output) | yes | |
| `list_organisations` | `GET /organizations` | yes | |
| `list_groups` | `GET /organizations/{o}/groups` | yes | |
| `list_group_members` | `GET /organizations/{o}/groups/{g}/members` | yes | |
| `list_webhooks` | `GET /webhooks` | yes | |
| `list_security_events` | `GET /seawatch/v1/security-events` (newest first; `event_types`, `status`, `actor_id`, `target_type`, `from_timestamp`, `to_timestamp`, `ip_address`) | yes | |
| `list_user_sessions` | `GET /users/{u}/sessions` | yes | |
| `create_user` | `POST /users` | no | no (additive) |
| `update_user` | `GET /users/{u}` then `PUT /users/{u}` with the given fields merged into the current ones | no | no |
| `disable_user` | `GET /users/{u}` then `PUT /users/{u}` with `enabled: false` | no | yes (revokes the user's tokens) |
| `enable_user` | `GET /users/{u}` then `PUT /users/{u}` with `enabled: true` | no | no |
| `require_user_actions` | `GET /users/{u}` then `PUT /users/{u}` with `required_actions` (replaces the list; `update_password` forces a password reset at next sign-in) | no | yes (replaces) |
| `unlock_user` | `POST /users/{u}/unlock` | no | no |
| `assign_role` | `POST /users/{u}/roles/{r}` | no | no |
| `unassign_role` | `DELETE /users/{u}/roles/{r}` | no | yes |
| `add_group_member` | `POST /organizations/{o}/groups/{g}/members` | no | no |
| `remove_group_member` | `DELETE /organizations/{o}/groups/{g}/members/{u}` | no | yes |
| `revoke_user_session` | `DELETE /users/{u}/sessions/{s}` | no | yes |

FerrisKey's `PUT /users/{u}` replaces the whole record, so the three user-update tools read the user first and send back every field they were not asked to change.

Not offered, on purpose or because the API has no route for it:

- Setting a password (`PUT /users/{u}/reset-password`) would put a secret through the agent. Use `require_user_actions` with `update_password` instead. FerrisKey has no admin route that sends a reset email.
- Client secrets, webhook secrets and credentials are never read or returned.
- Deleting users, clients, roles, organisations or webhooks.
- Realm-wide groups: FerrisKey's groups belong to an organisation.
