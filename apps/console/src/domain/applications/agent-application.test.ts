import { describe, expect, it } from 'vitest'
import { isAgentApplication } from './agent-application'

describe('which applications the Agents tab made', () => {
  it('knows an agent- client id', () => {
    expect(isAgentApplication({ client_id: 'agent-n8n-ab12cd' })).toBe(true)
  })

  it.each(['web', 'my-agent-app', 'Agent-x', 'agent', 'app-agent-'])(
    'does not mistake %s for one',
    (clientId) => {
      expect(isAgentApplication({ client_id: clientId })).toBe(false)
    },
  )
})
