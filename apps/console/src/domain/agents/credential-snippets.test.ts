import { describe, expect, it } from 'vitest'
import { curlSnippet, n8nFields } from './credential-snippets'

const call = {
  tokenUrl: 'https://auth.acme.test/api/realms/acme/protocol/openid-connect/token',
  clientId: 'agent-nightly-abc123',
  secret: 's3cr\'et',
  audience: 'https://auth.acme.test/mcp',
}

describe('the credential snippets', () => {
  it('asks for a client credentials token for the MCP resource', () => {
    const curl = curlSnippet(call)

    expect(curl).toContain(`'${call.tokenUrl}'`)
    expect(curl).toContain('grant_type=client_credentials')
    expect(curl).toContain('client_id=\'agent-nightly-abc123\'')
    expect(curl).toContain('resource=\'https://auth.acme.test/mcp\'')
  })

  it('quotes a secret that holds a quote', () => {
    expect(curlSnippet(call)).toContain('client_secret=\'s3cr\'\\\'\'et\'')
  })

  it('lists the n8n OAuth2 credential fields', () => {
    const fields = Object.fromEntries(n8nFields(call).map((field) => [field.label, field.value]))

    expect(fields['Grant Type']).toBe('Client Credentials')
    expect(fields['Access Token URL']).toBe(call.tokenUrl)
    expect(fields['Client ID']).toBe(call.clientId)
    expect(fields['Client Secret']).toBe(call.secret)
    expect(fields['Resource (MCP server URL)']).toBe(call.audience)
  })
})
