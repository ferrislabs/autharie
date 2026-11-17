import { issuerOf } from './facts'

export const ADMIN_CLIENT_ID = 'autharie-admin'

export const REPLACE_WARNING =
  'Generating replaces any admin access issued before. The previous secret stops working.'

export interface AdminAccess {
  clientId: string
  clientSecret: string
}

export function parseAdminAccess(value: unknown): AdminAccess {
  const record = (typeof value === 'object' && value !== null ? value : {}) as Record<
    string,
    unknown
  >
  const id = record.client_id
  const secret = record.client_secret

  if (typeof id !== 'string' || id === '' || typeof secret !== 'string' || secret === '') {
    throw new Error('The deployment returned an admin access we could not read. Try again.')
  }

  return { clientId: id, clientSecret: secret }
}

export function tokenUrlOf(endpoint: string): string {
  return `${issuerOf(endpoint, 'ferriskey')}/protocol/openid-connect/token`
}

export function curlCommands(endpoint: string): string {
  const api = `${endpoint}/api`

  return [
    '# 1. Get an admin token',
    `TOKEN=$(curl -s '${tokenUrlOf(endpoint)}' \\`,
    '  -d grant_type=client_credentials \\',
    `  -d client_id=${ADMIN_CLIENT_ID} \\`,
    '  --data-urlencode "client_secret=$CLIENT_SECRET" | jq -r .access_token)',
    '',
    '# 2. Create the user',
    `USER_ID=$(curl -s -X POST '${api}/realms/master/users' \\`,
    '  -H "Authorization: Bearer $TOKEN" -H \'Content-Type: application/json\' \\',
    '  -d "$(jq -n --arg email "$EMAIL" \'{username: $email, email: $email, email_verified: true}\')" \\',
    '  | jq -r .data.id)',
    '',
    '# 3. Set its password',
    `curl -s -X PUT "${api}/realms/master/users/$USER_ID/reset-password" \\`,
    '  -H "Authorization: Bearer $TOKEN" -H \'Content-Type: application/json\' \\',
    '  -d "$(jq -n --arg value "$PASSWORD" \'{value: $value, temporary: false, credential_type: "password"}\')"',
    '',
    `# 4. Give it the ${ADMIN_CLIENT_ID} role`,
    `ROLE_ID=$(curl -s '${api}/realms/master/roles?qualified_name=${ADMIN_CLIENT_ID}&scope=realm' \\`,
    '  -H "Authorization: Bearer $TOKEN" | jq -r \'.data[0].id\')',
    `curl -s -X POST "${api}/realms/master/users/$USER_ID/roles/$ROLE_ID" \\`,
    '  -H "Authorization: Bearer $TOKEN"',
  ].join('\n')
}

export function canIssueAdminAccess(input: {
  kind: string
  running: boolean
  mayManage: boolean
  endpoint: string | undefined
}): boolean {
  return input.kind === 'ferriskey' && input.running && input.mayManage && !!input.endpoint
}
