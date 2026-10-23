import { describe, expect, it } from 'vitest'
import { describeServer, policiesReferring, toolCount } from './server-status'
import type { AgentPolicy, AgentServer, ServerDiscovery } from './types'

function server(over: Partial<AgentServer> = {}): AgentServer {
  return {
    id: 's1',
    slug: 'crm',
    name: 'CRM',
    url: 'https://tools.example.com/mcp',
    enabled: true,
    created_at: '',
    updated_at: '',
    ...over,
  }
}

function discovery(over: Partial<ServerDiscovery> = {}): ServerDiscovery {
  return { phase: 'Ready', tools: [], dropped: 0, discovered_at: '2026-10-23T10:00:00Z', ...over }
}

function policy(over: Partial<AgentPolicy>): AgentPolicy {
  return {
    id: 'p',
    name: 'p',
    enabled: true,
    cedar: 'permit (principal, action, resource);',
    created_at: '',
    updated_at: '',
    ...over,
  }
}

describe('the status of a customer server', () => {
  it('is off, then discovering, until the data plane reported', () => {
    expect(describeServer(server({ enabled: false })).label).toBe('Off')
    expect(describeServer(server()).label).toBe('Discovering tools')
  })

  it('names what the data plane found', () => {
    const chip = (phase: ServerDiscovery['phase'], message?: string) =>
      describeServer(server({ discovery: discovery({ phase, message }) }))

    expect(chip('Ready')).toEqual({ tone: 'success', label: 'Ready', detail: undefined })
    expect(chip('Unreachable', 'timed out')).toMatchObject({ tone: 'danger', detail: 'timed out' })
    expect(chip('Refused', 'not a public address')).toMatchObject({ tone: 'danger', label: 'Refused' })
    expect(chip('Invalid')).toMatchObject({ tone: 'warning' })
  })

  it('counts the tools of a server that answered, and those left out', () => {
    const tools = [{ name: 'a', read_only: true, destructive: false }]

    expect(toolCount(server())).toBeNull()
    expect(toolCount(server({ discovery: discovery({ tools }) }))).toBe('1 tool')
    expect(toolCount(server({ discovery: discovery({ tools: [...tools, ...tools], dropped: 3 }) }))).toBe(
      '2 tools, 3 left out',
    )
    expect(toolCount(server({ discovery: discovery({ phase: 'Unreachable', tools }) }))).toBeNull()
  })
})

describe('the policies that refer to a server', () => {
  const rule = (what: NonNullable<AgentPolicy['rule']>['what']): AgentPolicy['rule'] => ({
    version: 1,
    effect: 'permit',
    who: { kind: 'any-user' },
    agents: { kind: 'any' },
    what,
    when: null,
  })

  it('finds them by the server a rule names, in every kind of target', () => {
    const policies = [
      policy({ id: 'whole', rule: rule({ kind: 'server', server: 'crm' }) }),
      policy({ id: 'ro', rule: rule({ kind: 'read-only', server: 'crm' }) }),
      policy({ id: 'tools', rule: rule({ kind: 'tools', tools: ['crm/list_invoices'] }) }),
      policy({ id: 'other', rule: rule({ kind: 'server', server: 'ferriskey' }) }),
      policy({ id: 'all', rule: rule({ kind: 'read-only', server: null }) }),
      policy({ id: 'prefix', rule: rule({ kind: 'tools', tools: ['crm2/x'] }) }),
    ]

    expect(policiesReferring(policies, 'crm').map((found) => found.id)).toEqual(['whole', 'ro', 'tools'])
  })

  it('looks into the Cedar of a policy that was written by hand', () => {
    const policies = [
      policy({ id: 'by-server', cedar: 'permit (principal, action, resource) when { resource.server == "crm" };' }),
      policy({ id: 'by-tool', cedar: 'permit (principal, action, resource == FerrisKey::Tool::"crm/list_invoices");' }),
      policy({ id: 'plain' }),
    ]

    expect(policiesReferring(policies, 'crm').map((found) => found.id)).toEqual(['by-server', 'by-tool'])
  })
})
