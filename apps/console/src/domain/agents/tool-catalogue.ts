export type ServerId = 'ferriskey' | 'autharie'

export interface CatalogueServer {
  id: ServerId
  label: string
}

export interface CatalogueTool {
  server: ServerId
  name: string
  description: string
  readOnly: boolean
  destructive: boolean
}

export const SERVERS: CatalogueServer[] = [
  { id: 'ferriskey', label: 'FerrisKey' },
  { id: 'autharie', label: 'Autharie' },
]

const FERRISKEY_TOOLS: CatalogueTool[] = [
  { server: 'ferriskey', name: 'list_realms', description: 'List the realms the caller can see.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_users', description: 'Search users.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'get_user', description: 'Read one user.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'get_user_roles', description: 'List a user’s roles.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_roles', description: 'List roles.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_clients', description: 'List clients, secrets removed.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'get_client', description: 'Read one client, secrets removed.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_organisations', description: 'List organisations.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_groups', description: 'List an organisation’s groups.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_group_members', description: 'List a group’s members.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_webhooks', description: 'List webhooks.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_security_events', description: 'Search security events.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'list_user_sessions', description: 'List a user’s sessions.', readOnly: true, destructive: false },
  { server: 'ferriskey', name: 'create_user', description: 'Create a user.', readOnly: false, destructive: false },
  { server: 'ferriskey', name: 'update_user', description: 'Change a user’s fields.', readOnly: false, destructive: false },
  { server: 'ferriskey', name: 'enable_user', description: 'Enable a user.', readOnly: false, destructive: false },
  { server: 'ferriskey', name: 'unlock_user', description: 'Unlock a user.', readOnly: false, destructive: false },
  { server: 'ferriskey', name: 'assign_role', description: 'Give a user a role.', readOnly: false, destructive: false },
  { server: 'ferriskey', name: 'add_group_member', description: 'Add a user to a group.', readOnly: false, destructive: false },
  { server: 'ferriskey', name: 'disable_user', description: 'Disable a user and revoke their tokens.', readOnly: false, destructive: true },
  { server: 'ferriskey', name: 'require_user_actions', description: 'Replace a user’s required actions.', readOnly: false, destructive: true },
  { server: 'ferriskey', name: 'unassign_role', description: 'Take a role from a user.', readOnly: false, destructive: true },
  { server: 'ferriskey', name: 'remove_group_member', description: 'Remove a user from a group.', readOnly: false, destructive: true },
  { server: 'ferriskey', name: 'revoke_user_session', description: 'Revoke a user session.', readOnly: false, destructive: true },
]

const AUTHARIE_TOOLS: CatalogueTool[] = [
  { server: 'autharie', name: 'list_organisations', description: 'List the caller’s organisations.', readOnly: true, destructive: false },
  { server: 'autharie', name: 'list_deployments', description: 'List an organisation’s deployments.', readOnly: true, destructive: false },
  { server: 'autharie', name: 'get_deployment', description: 'Read a deployment’s status and metrics.', readOnly: true, destructive: false },
  { server: 'autharie', name: 'get_recent_logs', description: 'Search recent logs.', readOnly: true, destructive: false },
  { server: 'autharie', name: 'list_backups', description: 'List a deployment’s backups.', readOnly: true, destructive: false },
  { server: 'autharie', name: 'create_deployment', description: 'Create a deployment.', readOnly: false, destructive: false },
  { server: 'autharie', name: 'upgrade_deployment', description: 'Upgrade a deployment to another version.', readOnly: false, destructive: true },
  { server: 'autharie', name: 'list_webhooks', description: 'List a deployment’s webhooks.', readOnly: true, destructive: false },
  { server: 'autharie', name: 'create_webhook', description: 'Create a webhook.', readOnly: false, destructive: false },
  { server: 'autharie', name: 'delete_webhook', description: 'Delete a webhook.', readOnly: false, destructive: true },
]

export const TOOL_CATALOGUE: CatalogueTool[] = [...FERRISKEY_TOOLS, ...AUTHARIE_TOOLS]

export function toolId(tool: Pick<CatalogueTool, 'server' | 'name'>): string {
  return `${tool.server}/${tool.name}`
}

export function toolsOf(server: ServerId): CatalogueTool[] {
  return TOOL_CATALOGUE.filter((tool) => tool.server === server)
}

export function findTool(id: string): CatalogueTool | undefined {
  return TOOL_CATALOGUE.find((tool) => toolId(tool) === id)
}

export function serverOfTool(id: string): string {
  const slash = id.indexOf('/')
  return slash < 0 ? '' : id.slice(0, slash)
}

export function nameOfTool(id: string): string {
  const slash = id.indexOf('/')
  return slash < 0 ? id : id.slice(slash + 1)
}
