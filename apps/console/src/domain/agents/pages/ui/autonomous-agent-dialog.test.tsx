import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { FlowEvent } from '../../credential-flow'
import type { AgentClient } from '../../types'
import { AutonomousDialogBody } from './autonomous-agent-dialog'

const noop = () => {}

const ready: FlowEvent = {
  phase: 'ready',
  revealed: {
    clientId: 'agent-nightly-abc123',
    secret: 'the-one-time-secret',
    tokenUrl: 'https://auth.acme.test/api/realms/acme/protocol/openid-connect/token',
    audience: 'https://auth.acme.test/mcp',
  },
}

const created: AgentClient = {
  client_id: 'agent-nightly-abc123',
  name: 'Nightly',
  source: 'autharie',
  kind: 'autonomous',
  access: 'approved',
}

function render(
  event: FlowEvent | null,
  mode: React.ComponentProps<typeof AutonomousDialogBody>['mode'] = { kind: 'create' },
) {
  return renderToStaticMarkup(
    <AutonomousDialogBody
      mode={mode}
      event={event}
      onCreate={noop}
      onRotate={noop}
      onClose={noop}
    />,
  )
}

describe('the autonomous agent dialog', () => {
  it('asks for a name and a description before anything is generated', () => {
    const html = render(null)

    expect(html).toContain('Name')
    expect(html).toContain('Description')
    expect(html).toContain('Create agent')
    expect(html).not.toContain('Client secret')
  })

  it('shows progress while the key is generated and the data plane answers', () => {
    expect(render({ phase: 'generating' })).toContain('Generating a one-time key in your browser')
    expect(render({ phase: 'requesting' })).toContain('Asking the data plane')
    expect(render({ phase: 'waiting' })).toContain('Waiting for the data plane')
    expect(render({ phase: 'waiting' })).not.toContain('Create agent')
  })

  it('shows the client id and the secret once with copy buttons and ready-to-paste snippets', () => {
    const html = render(ready)

    expect(html).toContain('agent-nightly-abc123')
    expect(html).toContain('the-one-time-secret')
    expect(html).toContain('Copy client id')
    expect(html).toContain('Copy client secret')
    expect(html).toContain('This secret is shown once')
    expect(html).toContain('grant_type=client_credentials')
    expect(html).toContain('resource=')
    expect(html).toContain('https://auth.acme.test/mcp')
    expect(html).toContain('OAuth2 API credential')
    expect(html).toContain('Access Token URL')
  })

  it('shows a failure and offers to close', () => {
    const html = render({ phase: 'failed', message: 'The request expired.' })

    expect(html).toContain('The request expired.')
    expect(html).toContain('Close')
  })

  it('asks for confirmation before rotating and warns the old secret stops working', () => {
    const html = render(null, { kind: 'rotate', client: created })

    expect(html).toContain('Rotate secret')
    expect(html).not.toContain('Create agent')
  })

  it('uses theme tokens so it reads in light and dark mode', () => {
    expect(render(ready)).not.toMatch(/bg-white|text-black|bg-gray-/)
  })
})
