# autharie-mcp

An MCP server over the Autharie API. It sits behind the MCP gateway (private, never exposed) and serves the `/mcp` route in stateless streamable HTTP (MCP 2026-07-28).

Each request carries the token the gateway exchanged for it, whose audience is the Autharie API. Every tool forwards that token as `Authorization: Bearer ...` to the API, which decides what the caller may do. The server has no credentials of its own, keeps no session and never stores a token. A request without a bearer is refused with 401. The API's refusals come back as tool errors (`isError: true`) carrying the status and message, not as protocol errors.

## Configuration

| Flag | Variable | Default |
| --- | --- | --- |
| `--listen` | `AUTHARIE_MCP_LISTEN` | `0.0.0.0:8081` |
| `--api-url` | `AUTHARIE_MCP_API_URL` | `http://localhost:3333` |

`GET /healthz` answers 200 without a token.

## Tools

Console policies refer to a tool as `autharie/<tool>`. The names are stable.

| Tool | API route | Read-only | Destructive |
| --- | --- | --- | --- |
| `list_organisations` | `GET /users/@me/organisations` | yes | |
| `list_deployments` | `GET /organisations/{o}/deployments` | yes | |
| `get_deployment` | `GET /organisations/{o}/deployments/{d}`, `.../availability`, `.../usage-metrics/{metric}` | yes | |
| `get_recent_logs` | `GET /organisations/{o}/logs/search` | yes | |
| `list_backups` | `GET /organisations/{o}/deployments/{d}/backups` | yes | |
| `create_deployment` | `POST /organisations/{o}/deployments` | no | no (additive) |
| `upgrade_deployment` | `POST /organisations/{o}/deployments/{d}/upgrade` | no | yes |
| `list_webhooks` | `GET /organisations/{o}/deployments/{d}/webhooks` | yes | |
| `create_webhook` | `POST /organisations/{o}/deployments/{d}/webhooks` | no | no (additive) |
| `delete_webhook` | `DELETE /organisations/{o}/deployments/{d}/webhooks/{w}` | no | yes |

Identifiers are validated as UUIDs before any call, so an argument cannot change the path it is placed in.

Not offered, because the API has no route for them: stopping and starting a deployment.
