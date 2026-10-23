export interface TokenCall {
  tokenUrl: string
  clientId: string
  secret: string
  audience: string
}

function quote(value: string): string {
  return `'${value.replace(/'/g, '\'\\\'\'')}'`
}

export function curlSnippet(call: TokenCall): string {
  return [
    `curl -s -X POST ${quote(call.tokenUrl)} \\`,
    '  -d grant_type=client_credentials \\',
    `  --data-urlencode client_id=${quote(call.clientId)} \\`,
    `  --data-urlencode client_secret=${quote(call.secret)} \\`,
    `  --data-urlencode resource=${quote(call.audience)}`,
  ].join('\n')
}

export interface CredentialField {
  label: string
  value: string
}

export function n8nFields(call: TokenCall): CredentialField[] {
  return [
    { label: 'Grant Type', value: 'Client Credentials' },
    { label: 'Access Token URL', value: call.tokenUrl },
    { label: 'Client ID', value: call.clientId },
    { label: 'Client Secret', value: call.secret },
    { label: 'Authentication', value: 'Send credentials in body' },
    { label: 'Resource (MCP server URL)', value: call.audience },
  ]
}
