import { describe, expect, it } from 'vitest'
import * as cedar from '@cedar-policy/cedar-wasm/nodejs'
import { compileRule } from './compile-rule'
import { contextTime, isValid, simulate, validatePolicy, type NamedPolicy, type Scenario } from './cedar-engine'
import { catalogueOf } from './tool-catalogue'
import type { RuleModel } from './types'

function compiled(over: Partial<RuleModel>): string {
  const result = compileRule({
    version: 1,
    effect: 'permit',
    who: { kind: 'any-user' },
    agents: { kind: 'any' },
    what: { kind: 'read-only', server: null },
    when: null,
    ...over,
  })
  if (!result.ok) throw new Error(result.issues.join('; '))
  return result.cedar
}

const MONDAY_10H = new Date('2026-10-19T10:30:00Z')
const SUNDAY_10H = new Date('2026-10-18T10:30:00Z')

function userScenario(over: Partial<Scenario> = {}): Scenario {
  return {
    subject: { kind: 'user', id: 'u-1', groups: ['support'], roles: ['viewer'], agent: 'agent-x' },
    tool: 'autharie/list_backups',
    time: MONDAY_10H,
    ...over,
  }
}

function policy(id: string, cedarText: string): NamedPolicy {
  return { id, name: id, cedar: cedarText }
}

describe('validating against the schema', () => {
  it('accepts every shape the builder can produce', () => {
    const rules: Partial<RuleModel>[] = [
      {},
      { effect: 'forbid' },
      { who: { kind: 'groups', groups: ['a'] } },
      { who: { kind: 'groups', groups: ['a', 'b'] } },
      { who: { kind: 'roles', roles: ['a', 'b'] } },
      { who: { kind: 'users', users: ['a'] } },
      { who: { kind: 'users', users: ['a', 'b'] } },
      { who: { kind: 'agents', clientIds: ['a'] } },
      { who: { kind: 'agents', clientIds: ['a', 'b'] } },
      { agents: { kind: 'clients', clientIds: ['a'] } },
      { agents: { kind: 'clients', clientIds: ['a', 'b'] } },
      { what: { kind: 'tools', tools: ['autharie/list_backups'] } },
      { what: { kind: 'tools', tools: ['autharie/list_backups', 'ferriskey/get_user'] } },
      { what: { kind: 'server', server: 'ferriskey' } },
      { what: { kind: 'read-only', server: 'autharie' } },
      { when: { weekdays: [1, 2, 3], hours: { from: 8, to: 18 } } },
      { when: { weekdays: [], hours: { from: 22, to: 6 } } },
    ]
    for (const rule of rules) {
      const text = compiled(rule)
      const validation = validatePolicy(cedar, text)
      expect(validation.syntax, text).toEqual([])
      expect(validation.errors, text).toEqual([])
    }
  })

  it('rejects an unguarded access to an optional attribute', () => {
    const validation = validatePolicy(
      cedar,
      'permit (principal, action, resource is FerrisKey::Tool) when { resource.annotations.readOnlyHint };',
    )
    expect(isValid(validation)).toBe(false)
  })

  it('rejects the arguments context FerrisKey does not provide', () => {
    const validation = validatePolicy(
      cedar,
      'permit (principal, action == FerrisKey::Action::"call_tool", resource) when { context.arguments.x };',
    )
    expect(isValid(validation)).toBe(false)
  })

  it('reports a syntax error', () => {
    const validation = validatePolicy(cedar, 'permit (principal, action, resource')
    expect(validation.syntax.length).toBeGreaterThan(0)
    expect(isValid(validation)).toBe(false)
  })

  it('reports an unknown entity type', () => {
    const validation = validatePolicy(
      cedar,
      'permit (principal is FerrisKey::Robot, action, resource);',
    )
    expect(isValid(validation)).toBe(false)
    expect(validation.errors.length).toBeGreaterThan(0)
  })

  it('reports an unknown attribute', () => {
    const validation = validatePolicy(
      cedar,
      'permit (principal, action, resource) when { resource.colour == "red" };',
    )
    expect(isValid(validation)).toBe(false)
  })

  it('accepts a policy over the context', () => {
    const validation = validatePolicy(
      cedar,
      'permit (principal, action == FerrisKey::Action::"call_tool", resource) when { context.delegated };',
    )
    expect(isValid(validation)).toBe(true)
  })
})

describe('simulating', () => {
  it('denies when no policy applies', () => {
    const outcome = simulate(cedar, [], userScenario())
    expect(outcome.decision).toBe('deny')
    expect(outcome.determining).toEqual([])
  })

  it('allows a read-only tool for a group and names the policy', () => {
    const permit = policy('p1', compiled({ who: { kind: 'groups', groups: ['support'] } }))
    const outcome = simulate(cedar, [permit], userScenario())
    expect(outcome.decision).toBe('allow')
    expect(outcome.determining.map((item) => item.id)).toEqual(['p1'])
  })

  it('denies a tool that is not read-only', () => {
    const permit = policy('p1', compiled({}))
    const outcome = simulate(cedar, [permit], userScenario({ tool: 'autharie/upgrade_deployment' }))
    expect(outcome.decision).toBe('deny')
  })

  it('denies a user outside the group', () => {
    const permit = policy('p1', compiled({ who: { kind: 'groups', groups: ['finance'] } }))
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('deny')
  })

  it('matches a role', () => {
    const permit = policy('p1', compiled({ who: { kind: 'roles', roles: ['viewer'] } }))
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('allow')
  })

  it('matches a named agent and refuses another', () => {
    const permit = policy('p1', compiled({ agents: { kind: 'clients', clientIds: ['agent-x'] } }))
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('allow')
    const other = userScenario({
      subject: { kind: 'user', id: 'u-1', groups: [], roles: [], agent: 'agent-y' },
    })
    expect(simulate(cedar, [permit], other).decision).toBe('deny')
  })

  it('lets a forbid win over a permit', () => {
    const permit = policy('allow', compiled({}))
    const forbid = policy(
      'block',
      compiled({ effect: 'forbid', what: { kind: 'tools', tools: ['autharie/list_backups'] } }),
    )
    const outcome = simulate(cedar, [permit, forbid], userScenario())
    expect(outcome.decision).toBe('deny')
    expect(outcome.determining.map((item) => item.id)).toEqual(['block'])
  })

  it('honours the weekday and hour window in UTC', () => {
    const permit = policy(
      'p1',
      compiled({ when: { weekdays: [1, 2, 3, 4, 5], hours: { from: 8, to: 18 } } }),
    )
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('allow')
    expect(simulate(cedar, [permit], userScenario({ time: SUNDAY_10H })).decision).toBe('deny')
    expect(
      simulate(cedar, [permit], userScenario({ time: new Date('2026-10-19T19:00:00Z') })).decision,
    ).toBe('deny')
  })

  it('honours a window that crosses midnight', () => {
    const permit = policy('p1', compiled({ when: { weekdays: [], hours: { from: 22, to: 6 } } }))
    expect(simulate(cedar, [permit], userScenario({ time: new Date('2026-10-19T23:00:00Z') })).decision).toBe('allow')
    expect(simulate(cedar, [permit], userScenario({ time: new Date('2026-10-19T03:00:00Z') })).decision).toBe('allow')
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('deny')
  })

  it('simulates an autonomous agent', () => {
    const permit = policy('p1', compiled({ who: { kind: 'agents', clientIds: ['bot'] } }))
    const scenario = userScenario({ subject: { kind: 'agent', clientId: 'bot' } })
    expect(simulate(cedar, [permit], scenario).decision).toBe('allow')
    const stranger = userScenario({ subject: { kind: 'agent', clientId: 'other' } })
    expect(simulate(cedar, [permit], stranger).decision).toBe('deny')
  })

  it('does not let a user match an autonomous-agent rule', () => {
    const permit = policy('p1', compiled({ who: { kind: 'agents', clientIds: ['agent-x'] } }))
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('deny')
  })

  it('matches a tool of a server', () => {
    const permit = policy('p1', compiled({ what: { kind: 'server', server: 'ferriskey' } }))
    expect(simulate(cedar, [permit], userScenario({ tool: 'ferriskey/disable_user' })).decision).toBe('allow')
    expect(simulate(cedar, [permit], userScenario()).decision).toBe('deny')
  })
})

describe('the time handed to a policy', () => {
  it('numbers Monday 1 and Sunday 7', () => {
    expect(contextTime(MONDAY_10H)).toEqual({ weekday: 1, hour: 10 })
    expect(contextTime(SUNDAY_10H)).toEqual({ weekday: 7, hour: 10 })
  })
})

describe('the tools of a customer server', () => {
  const catalogue = catalogueOf([
    {
      slug: 'crm',
      name: 'CRM',
      discovery: {
        tools: [
          { name: 'list_invoices', read_only: true, destructive: false },
          { name: 'delete_invoice', read_only: false, destructive: true },
        ],
      },
    },
  ])
  const supportOnCrm = compiled({
    who: { kind: 'groups', groups: ['support'] },
    what: { kind: 'read-only', server: 'crm' },
  })
  const asSupport = (tool: string, groups = ['support']) =>
    userScenario({
      subject: { kind: 'user', id: 'u-1', groups, roles: [], agent: 'agent-x' },
      tool,
    })

  it('is checked against the schema', () => {
    const validation = validatePolicy(cedar, supportOnCrm)

    expect(validation.syntax).toEqual([])
    expect(validation.errors).toEqual([])
  })

  it('is denied by default, then allowed by the rule for a read-only tool', () => {
    const none: NamedPolicy[] = []
    const some = [policy('crm-reads', supportOnCrm)]

    expect(simulate(cedar, none, asSupport('crm/list_invoices'), catalogue).decision).toBe('deny')
    const outcome = simulate(cedar, some, asSupport('crm/list_invoices'), catalogue)
    expect(outcome.decision).toBe('allow')
    expect(outcome.determining.map((entry) => entry.id)).toEqual(['crm-reads'])
  })

  it('keeps denying what the rule does not cover', () => {
    const some = [policy('crm-reads', supportOnCrm)]

    expect(simulate(cedar, some, asSupport('crm/delete_invoice'), catalogue).decision).toBe('deny')
    expect(simulate(cedar, some, asSupport('crm/list_invoices', ['sales']), catalogue).decision).toBe('deny')
    expect(simulate(cedar, some, asSupport('ferriskey/get_user'), catalogue).decision).toBe('deny')
  })

  it('takes the read-only hint the server announced', () => {
    const some = [policy('crm-reads', supportOnCrm)]

    expect(simulate(cedar, some, asSupport('crm/list_invoices')).decision).toBe('deny')
  })

  it('is decided like any tool, even one missing from the last discovery', () => {
    const permitAll = [policy('server', compiled({ what: { kind: 'server', server: 'crm' } }))]

    expect(simulate(cedar, permitAll, asSupport('crm/brand_new_tool'), catalogue).decision).toBe('allow')
  })
})
