import { describe, expect, it } from 'vitest'
import { ORGANISATION_OFF_NOTE, describeAgents } from './agent-state'
import type { AgentSettings } from './types'

const base: AgentSettings = {
  enabled: false,
  realm: '',
  organisation_mcp_enabled: true,
}

describe('the agents header', () => {
  it('explains that AI access is off for the organisation and locks the switch', () => {
    const state = describeAgents({ ...base, organisation_mcp_enabled: false }, true)
    expect(state.kind).toBe('organisation-off')
    expect(state.switchDisabled).toBe(true)
    expect(state.callout).toBe(ORGANISATION_OFF_NOTE)
    expect(state.badge).toEqual({ tone: 'neutral', label: 'Off for the organisation' })
  })

  it('locks the switch for the organisation even when the deployment had agents on', () => {
    const state = describeAgents(
      { ...base, enabled: true, realm: 'master', organisation_mcp_enabled: false },
      true,
    )
    expect(state.kind).toBe('organisation-off')
    expect(state.switchDisabled).toBe(true)
  })

  it('shows a disabled deployment as off with the switch free', () => {
    const state = describeAgents(base, true)
    expect(state.kind).toBe('disabled')
    expect(state.switchDisabled).toBe(false)
    expect(state.callout).toBeNull()
    expect(state.badge).toEqual({ tone: 'neutral', label: 'Off' })
  })

  it('locks the switch for somebody who cannot manage', () => {
    expect(describeAgents(base, false).switchDisabled).toBe(true)
  })

  it('shows an enabled deployment with no report yet as waiting', () => {
    const state = describeAgents({ ...base, enabled: true, realm: 'master' }, true)
    expect(state.kind).toBe('applying')
    expect(state.badge.label).toBe('Waiting for the data plane')
  })

  it('shows Applying as progress', () => {
    const state = describeAgents(
      {
        ...base,
        enabled: true,
        realm: 'master',
        status: { phase: 'Applying', gateway_client_ready: false, updated_at: '' },
      },
      true,
    )
    expect(state.kind).toBe('applying')
    expect(state.badge).toEqual({ tone: 'progress', label: 'Applying' })
    expect(state.gateway).toBe('Not ready')
  })

  it('shows Ready as success with the gateway client ready', () => {
    const state = describeAgents(
      {
        ...base,
        enabled: true,
        realm: 'master',
        mcp_url: 'https://x/mcp',
        status: { phase: 'Ready', gateway_client_ready: true, updated_at: '' },
      },
      true,
    )
    expect(state.kind).toBe('ready')
    expect(state.badge).toEqual({ tone: 'success', label: 'Ready' })
    expect(state.gateway).toBe('Ready')
  })

  it('shows Failed as danger with the message', () => {
    const state = describeAgents(
      {
        ...base,
        enabled: true,
        realm: 'master',
        status: { phase: 'Failed', gateway_client_ready: false, message: 'realm not found', updated_at: '' },
      },
      true,
    )
    expect(state.kind).toBe('failed')
    expect(state.badge).toEqual({ tone: 'danger', label: 'Failed' })
    expect(state.message).toBe('realm not found')
  })

  it('does not show a stale status while the deployment is off', () => {
    const state = describeAgents(
      {
        ...base,
        status: { phase: 'Ready', gateway_client_ready: true, updated_at: '' },
      },
      true,
    )
    expect(state.kind).toBe('disabled')
  })
})
