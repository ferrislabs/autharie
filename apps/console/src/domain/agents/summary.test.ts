import { describe, expect, it } from 'vitest'
import { describeRule, effectOfCedar, summariseHours, summariseWeekdays, summarisePolicy } from './summary'
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

describe('weekdays in words', () => {
  it('collapses a run', () => {
    expect(summariseWeekdays([1, 2, 3, 4, 5])).toBe('Mon–Fri')
  })

  it('lists separate days', () => {
    expect(summariseWeekdays([1, 3, 7])).toBe('Mon, Wed, Sun')
  })

  it('mixes a run and a day', () => {
    expect(summariseWeekdays([1, 2, 3, 6])).toBe('Mon–Wed, Sat')
  })

  it('says every day for all seven', () => {
    expect(summariseWeekdays([1, 2, 3, 4, 5, 6, 7])).toBe('every day')
  })

  it('keeps a pair as a list', () => {
    expect(summariseWeekdays([6, 7])).toBe('Sat, Sun')
  })
})

describe('hours in words', () => {
  it('pads and says UTC', () => {
    expect(summariseHours({ from: 8, to: 18 })).toBe('08:00–18:00 UTC')
  })

  it('writes midnight as 24:00', () => {
    expect(summariseHours({ from: 20, to: 24 })).toBe('20:00–24:00 UTC')
  })
})

describe('a rule in one line', () => {
  it('describes the simplest rule', () => {
    expect(describeRule(rule())).toBe('Permit any user to call read-only tools')
  })

  it('describes groups, agents, tools and time', () => {
    expect(
      describeRule(
        rule({
          who: { kind: 'groups', groups: ['support'] },
          agents: { kind: 'clients', clientIds: ['agent-x'] },
          what: { kind: 'tools', tools: ['autharie/list_backups'] },
          when: { weekdays: [1, 2, 3, 4, 5], hours: { from: 8, to: 18 } },
        }),
      ),
    ).toBe(
      'Permit users in group support to call autharie/list_backups through agent-x, Mon–Fri, 08:00–18:00 UTC',
    )
  })

  it('describes a forbid on a whole server', () => {
    expect(
      describeRule(rule({ effect: 'forbid', what: { kind: 'server', server: 'ferriskey' } })),
    ).toBe('Forbid any user to call all ferriskey tools')
  })

  it('counts several tools', () => {
    expect(
      describeRule(rule({ what: { kind: 'tools', tools: ['a/b', 'a/c', 'a/d'] } })),
    ).toBe('Permit any user to call 3 tools')
  })

  it('describes roles, users and autonomous agents', () => {
    expect(describeRule(rule({ who: { kind: 'roles', roles: ['admin', 'ops'] } }))).toContain('users with roles admin, ops')
    expect(describeRule(rule({ who: { kind: 'users', users: ['u-1'] } }))).toContain('user u-1')
    expect(describeRule(rule({ who: { kind: 'agents', clientIds: ['bot'] } }))).toContain('autonomous agent bot')
  })

  it('names read-only tools of a server', () => {
    expect(describeRule(rule({ what: { kind: 'read-only', server: 'autharie' } }))).toContain(
      'read-only autharie tools',
    )
  })
})

describe('the effect of raw Cedar', () => {
  it('reads permit', () => {
    expect(effectOfCedar('permit (principal, action, resource);')).toBe('permit')
  })

  it('reads forbid behind annotations', () => {
    expect(effectOfCedar('@id("x")\nforbid (principal, action, resource);')).toBe('forbid')
  })

  it('gives nothing for text that is neither', () => {
    expect(effectOfCedar('nonsense')).toBeNull()
  })
})

describe('a policy in one line', () => {
  const base: AgentPolicy = {
    id: 'p',
    name: 'n',
    enabled: true,
    cedar: 'forbid (principal, action, resource);',
    created_at: '',
    updated_at: '',
  }

  it('describes the rule when the policy has one', () => {
    expect(summarisePolicy({ ...base, rule: rule() })).toBe('Permit any user to call read-only tools')
  })

  it('says custom Cedar otherwise', () => {
    expect(summarisePolicy(base)).toBe('Forbid · custom Cedar')
  })

  it('says custom Cedar for unreadable text', () => {
    expect(summarisePolicy({ ...base, cedar: 'x' })).toBe('Custom Cedar')
  })
})
