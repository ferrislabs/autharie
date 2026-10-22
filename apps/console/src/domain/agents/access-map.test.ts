import { describe, expect, it } from 'vitest'
import {
  buildAccessGraph,
  COLUMN_X,
  connectedPolicies,
  ROW_GAP,
  ruleFromConnection,
} from './access-map'
import { compileRule } from './compile-rule'
import type { AgentPolicy, RuleModel } from './types'

function rule(over: Partial<RuleModel> = {}): RuleModel {
  return {
    version: 1,
    effect: 'permit',
    who: { kind: 'any-user' },
    agents: { kind: 'any' },
    what: { kind: 'read-only', server: null },
    when: null,
    ...over,
  }
}

function policy(id: string, over: Partial<AgentPolicy> = {}): AgentPolicy {
  return {
    id,
    name: id,
    enabled: true,
    cedar: 'permit (principal, action, resource);',
    created_at: '',
    updated_at: '',
    rule: rule(),
    ...over,
  }
}

describe('buildAccessGraph', () => {
  it('draws one path per rule policy', () => {
    const graph = buildAccessGraph([
      policy('p1', {
        rule: rule({
          who: { kind: 'roles', roles: ['admin'] },
          what: { kind: 'tools', tools: ['ferriskey/delete_user'] },
        }),
      }),
    ])
    expect(graph.nodes.map((n) => n.id).sort()).toEqual([
      'agent:any',
      'what:tool:ferriskey/delete_user',
      'who:role:admin',
    ])
    expect(graph.edges.map((e) => [e.id, e.source, e.target])).toEqual([
      ['p1:1', 'who:role:admin', 'agent:any'],
      ['p1:2', 'agent:any', 'what:tool:ferriskey/delete_user'],
    ])
    expect(graph.unmapped).toEqual([])
  })

  it('deduplicates nodes across policies', () => {
    const graph = buildAccessGraph([policy('a'), policy('b', { rule: rule({ effect: 'forbid' }) })])
    expect(graph.nodes).toHaveLength(3)
    expect(graph.edges).toHaveLength(4)
  })

  it('carries effect, enabled and the time window on the agent to what edge', () => {
    const graph = buildAccessGraph([
      policy('p', {
        enabled: false,
        rule: rule({ effect: 'forbid', when: { weekdays: [1, 2, 3, 4, 5], hours: { from: 9, to: 17 } } }),
      }),
    ])
    const [first, second] = graph.edges
    expect(first.data).toMatchObject({ policyId: 'p', effect: 'forbid', enabled: false, label: null })
    expect(second.data.label).toBe('Mon–Fri, 09:00–17:00 UTC')
  })

  it('starts autonomous agents at the agent column', () => {
    const graph = buildAccessGraph([
      policy('p', { rule: rule({ who: { kind: 'agents', clientIds: ['bot'] } }) }),
    ])
    expect(graph.nodes.map((n) => n.id).sort()).toEqual(['agent:client:bot', 'what:read-only:all'])
    expect(graph.nodes.find((n) => n.id === 'agent:client:bot')?.data.caption).toBe('acts on its own')
    expect(graph.edges.map((e) => e.id)).toEqual(['p:2'])
  })

  it('expands lists into several nodes with unique edge ids', () => {
    const graph = buildAccessGraph([
      policy('p', { rule: rule({ who: { kind: 'groups', groups: ['a', 'b'] } }) }),
    ])
    const ids = graph.edges.map((e) => e.id)
    expect(new Set(ids).size).toBe(ids.length)
    expect(ids).toHaveLength(3)
    expect(ids[0]).toBe('p:1')
  })

  it('labels tools and marks destructive ones', () => {
    const graph = buildAccessGraph([
      policy('p', {
        rule: rule({ what: { kind: 'tools', tools: ['ferriskey/disable_user', 'ferriskey/get_user'] } }),
      }),
    ])
    expect(graph.nodes.find((n) => n.id === 'what:tool:ferriskey/disable_user')?.data.destructive).toBe(true)
    expect(graph.nodes.find((n) => n.id === 'what:tool:ferriskey/get_user')?.data.destructive).toBe(false)
  })

  it('names servers and read-only scopes', () => {
    const graph = buildAccessGraph([
      policy('a', { rule: rule({ what: { kind: 'server', server: 'ferriskey' } }) }),
      policy('b', { rule: rule({ what: { kind: 'read-only', server: 'autharie' } }) }),
    ])
    expect(graph.nodes.find((n) => n.id === 'what:server:ferriskey')?.data.label).toBe('FerrisKey')
    expect(graph.nodes.find((n) => n.id === 'what:read-only:autharie')?.data.caption).toContain('read-only')
  })

  it('puts policies without a rule in unmapped', () => {
    const raw = policy('raw', { rule: undefined })
    const graph = buildAccessGraph([raw])
    expect(graph.unmapped).toEqual([raw])
    expect(graph.nodes).toEqual([])
    expect(graph.edges).toEqual([])
  })

  it('lays out fixed columns with any first then alphabetical', () => {
    const graph = buildAccessGraph([
      policy('a', { rule: rule({ who: { kind: 'roles', roles: ['zed', 'alpha'] } }) }),
      policy('b'),
    ])
    const who = graph.nodes.filter((n) => n.type === 'who')
    expect(who.map((n) => n.id)).toEqual(['who:any', 'who:role:alpha', 'who:role:zed'])
    expect(who.map((n) => n.position)).toEqual([
      { x: COLUMN_X.who, y: 0 },
      { x: COLUMN_X.who, y: ROW_GAP },
      { x: COLUMN_X.who, y: ROW_GAP * 2 },
    ])
    expect(graph.nodes.find((n) => n.type === 'agent')?.position.x).toBe(COLUMN_X.agent)
    expect(graph.nodes.find((n) => n.type === 'what')?.position.x).toBe(COLUMN_X.what)
  })

  it('is deterministic whatever the policy order', () => {
    const a = policy('a', { rule: rule({ who: { kind: 'users', users: ['u'] } }) })
    const b = policy('b')
    const one = buildAccessGraph([a, b]).nodes.map((n) => [n.id, n.position])
    const two = buildAccessGraph([b, a]).nodes.map((n) => [n.id, n.position])
    expect(one).toEqual(two)
  })
})

describe('connectedPolicies', () => {
  const graph = buildAccessGraph([
    policy('a', { rule: rule({ who: { kind: 'roles', roles: ['admin'] } }) }),
    policy('b'),
  ])

  it('lists the policies touching a node', () => {
    expect(connectedPolicies(graph, 'who:role:admin')).toEqual(['a'])
    expect(connectedPolicies(graph, 'agent:any').sort()).toEqual(['a', 'b'])
  })

  it('is empty for an unknown node', () => {
    expect(connectedPolicies(graph, 'nope')).toEqual([])
  })
})

describe('ruleFromConnection', () => {
  function compiles(model: RuleModel | null): RuleModel {
    if (model === null) throw new Error('no rule')
    expect(compileRule(model).ok).toBe(true)
    return model
  }

  it('builds a permit from a who to a tool', () => {
    const model = compiles(ruleFromConnection('who:group:ops', 'what:tool:ferriskey/list_users'))
    expect(model).toMatchObject({
      effect: 'permit',
      who: { kind: 'groups', groups: ['ops'] },
      agents: { kind: 'any' },
      what: { kind: 'tools', tools: ['ferriskey/list_users'] },
      when: null,
    })
  })

  it('handles every who and what kind', () => {
    expect(compiles(ruleFromConnection('who:any', 'what:read-only:all')).what).toEqual({
      kind: 'read-only',
      server: null,
    })
    expect(compiles(ruleFromConnection('who:role:r', 'what:read-only:autharie')).what).toEqual({
      kind: 'read-only',
      server: 'autharie',
    })
    expect(compiles(ruleFromConnection('who:user:u', 'what:server:ferriskey')).who).toEqual({
      kind: 'users',
      users: ['u'],
    })
  })

  it('keeps colons inside names', () => {
    const model = compiles(ruleFromConnection('who:user:a:b', 'what:server:ferriskey'))
    expect(model.who).toEqual({ kind: 'users', users: ['a:b'] })
  })

  it('uses the agent node as the agent', () => {
    const model = compiles(ruleFromConnection('agent:client:cli', 'what:server:ferriskey'))
    expect(model.who).toEqual({ kind: 'any-user' })
    expect(model.agents).toEqual({ kind: 'clients', clientIds: ['cli'] })
    expect(compiles(ruleFromConnection('agent:any', 'what:server:ferriskey')).agents).toEqual({ kind: 'any' })
  })

  it('refuses connections that name no tool scope', () => {
    expect(ruleFromConnection('who:any', 'agent:any')).toBeNull()
    expect(ruleFromConnection('what:server:x', 'what:server:y')).toBeNull()
    expect(ruleFromConnection('who:any', 'what:bogus:x')).toBeNull()
  })
})
