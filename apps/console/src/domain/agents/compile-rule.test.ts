import { describe, expect, it } from 'vitest'
import { compileRule, ruleIssues } from './compile-rule'
import type { RuleModel } from './types'

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

function cedar(over: Partial<RuleModel> = {}): string {
  const result = compileRule(rule(over))
  if (!result.ok) throw new Error(result.issues.join('; '))
  return result.cedar
}

describe('the head of a policy', () => {
  it('compiles the simplest rule', () => {
    expect(cedar()).toBe(
      [
        'permit (',
        '  principal is FerrisKey::User,',
        '  action == FerrisKey::Action::"call_tool",',
        '  resource is FerrisKey::Tool',
        ')',
        'when { resource has annotations && resource.annotations has readOnlyHint && resource.annotations.readOnlyHint };',
      ].join('\n'),
    )
  })

  it('uses forbid for a forbid rule', () => {
    expect(cedar({ effect: 'forbid' }).startsWith('forbid (')).toBe(true)
  })
})

describe('who', () => {
  it('puts a single group in the scope', () => {
    const out = cedar({ who: { kind: 'groups', groups: ['support'] } })
    expect(out).toContain('principal is FerrisKey::User in FerrisKey::Group::"support",')
  })

  it('moves several groups to a condition', () => {
    const out = cedar({ who: { kind: 'groups', groups: ['support', 'ops'] } })
    expect(out).toContain('  principal is FerrisKey::User,')
    expect(out).toContain(
      'when { principal in FerrisKey::Group::"support" || principal in FerrisKey::Group::"ops" }',
    )
  })

  it('tests roles on the user', () => {
    const out = cedar({ who: { kind: 'roles', roles: ['admin'] } })
    expect(out).toContain('when { principal has roles && principal.roles.contains("admin") }')
  })

  it('accepts any of several roles', () => {
    const out = cedar({ who: { kind: 'roles', roles: ['admin', 'ops'] } })
    expect(out).toContain(
      'when { principal has roles && (principal.roles.contains("admin") || principal.roles.contains("ops")) }',
    )
  })

  it('puts a single user in the scope', () => {
    const out = cedar({ who: { kind: 'users', users: ['u-1'] } })
    expect(out).toContain('principal == FerrisKey::User::"u-1",')
  })

  it('moves several users to a condition', () => {
    const out = cedar({ who: { kind: 'users', users: ['u-1', 'u-2'] } })
    expect(out).toContain('when { principal == FerrisKey::User::"u-1" || principal == FerrisKey::User::"u-2" }')
  })

  it('targets an autonomous agent as the principal', () => {
    const out = cedar({ who: { kind: 'agents', clientIds: ['bot'] } })
    expect(out).toContain('principal == FerrisKey::Agent::"bot",')
  })

  it('moves several autonomous agents to a condition', () => {
    const out = cedar({ who: { kind: 'agents', clientIds: ['a', 'b'] } })
    expect(out).toContain('  principal is FerrisKey::Agent,')
    expect(out).toContain('when { principal == FerrisKey::Agent::"a" || principal == FerrisKey::Agent::"b" }')
  })
})

describe('which agents', () => {
  it('adds nothing for any agent', () => {
    expect(cedar()).not.toContain('context.agent')
  })

  it('compares one client id', () => {
    const out = cedar({ agents: { kind: 'clients', clientIds: ['claude'] } })
    expect(out).toContain('when { context.agent == "claude" }')
  })

  it('tests membership of several client ids', () => {
    const out = cedar({ agents: { kind: 'clients', clientIds: ['a', 'b'] } })
    expect(out).toContain('when { ["a", "b"].contains(context.agent) }')
  })
})

describe('what', () => {
  it('names a single tool in the scope', () => {
    const out = cedar({ what: { kind: 'tools', tools: ['ferriskey/list_users'] } })
    expect(out).toContain('resource == FerrisKey::Tool::"ferriskey/list_users"')
    expect(out).not.toContain('when')
  })

  it('moves several tools to a condition', () => {
    const out = cedar({ what: { kind: 'tools', tools: ['autharie/list_backups', 'ferriskey/get_user'] } })
    expect(out).toContain('  resource is FerrisKey::Tool')
    expect(out).toContain(
      'when { resource == FerrisKey::Tool::"autharie/list_backups" || resource == FerrisKey::Tool::"ferriskey/get_user" }',
    )
  })

  it('selects every tool of a server', () => {
    const out = cedar({ what: { kind: 'server', server: 'autharie' } })
    expect(out).toContain('when { resource.server == "autharie" }')
  })

  it('selects read-only tools of one server', () => {
    const out = cedar({ what: { kind: 'read-only', server: 'ferriskey' } })
    expect(out).toContain('when { resource.server == "ferriskey" }')
    expect(out).toContain('resource.annotations has readOnlyHint')
  })
})

describe('when', () => {
  it('lists weekdays as a set', () => {
    const out = cedar({ when: { weekdays: [5, 1, 3, 3], hours: null } })
    expect(out).toContain('when { [1, 3, 5].contains(context.time.weekday) }')
  })

  it('adds nothing when every weekday is picked and there is no hour window', () => {
    const out = cedar({ when: { weekdays: [1, 2, 3, 4, 5, 6, 7], hours: null } })
    expect(out).not.toContain('context.time')
  })

  it('bounds a daytime window on both ends', () => {
    const out = cedar({ when: { weekdays: [], hours: { from: 8, to: 18 } } })
    expect(out).toContain('when { context.time.hour >= 8 && context.time.hour < 18 }')
  })

  it('drops the upper bound when the window runs to midnight', () => {
    const out = cedar({ when: { weekdays: [], hours: { from: 20, to: 24 } } })
    expect(out).toContain('when { context.time.hour >= 20 }')
  })

  it('wraps a window that crosses midnight', () => {
    const out = cedar({ when: { weekdays: [], hours: { from: 22, to: 6 } } })
    expect(out).toContain('when { context.time.hour >= 22 || context.time.hour < 6 }')
  })

  it('adds nothing for a whole day', () => {
    const out = cedar({ when: { weekdays: [], hours: { from: 0, to: 24 } } })
    expect(out).not.toContain('context.time')
  })

  it('combines weekdays and hours as separate conditions', () => {
    const out = cedar({ when: { weekdays: [1, 2, 3, 4, 5], hours: { from: 8, to: 18 } } })
    expect(out).toContain('when { [1, 2, 3, 4, 5].contains(context.time.weekday) }')
    expect(out).toContain('when { context.time.hour >= 8 && context.time.hour < 18 }')
  })
})

describe('the whole spec example', () => {
  it('compiles agent X reading tools for support on weekdays 8-18', () => {
    expect(
      cedar({
        who: { kind: 'groups', groups: ['support'] },
        agents: { kind: 'clients', clientIds: ['agent-x'] },
        what: { kind: 'read-only', server: null },
        when: { weekdays: [1, 2, 3, 4, 5], hours: { from: 8, to: 18 } },
      }),
    ).toBe(
      [
        'permit (',
        '  principal is FerrisKey::User in FerrisKey::Group::"support",',
        '  action == FerrisKey::Action::"call_tool",',
        '  resource is FerrisKey::Tool',
        ')',
        'when { context.agent == "agent-x" }',
        'when { resource has annotations && resource.annotations has readOnlyHint && resource.annotations.readOnlyHint }',
        'when { [1, 2, 3, 4, 5].contains(context.time.weekday) }',
        'when { context.time.hour >= 8 && context.time.hour < 18 };',
      ].join('\n'),
    )
  })
})

describe('quoting', () => {
  it('escapes quotes and backslashes in identifiers', () => {
    const out = cedar({ who: { kind: 'groups', groups: ['a"b\\c'] } })
    expect(out).toContain('FerrisKey::Group::"a\\"b\\\\c"')
  })

  it('escapes control characters', () => {
    const out = cedar({ who: { kind: 'roles', roles: ['a\nb'] } })
    expect(out).toContain('"a\\u{a}b"')
  })

  it('cannot be broken out of by an identifier', () => {
    const out = cedar({ agents: { kind: 'clients', clientIds: ['x" || true || "'] } })
    expect(out).toContain('context.agent == "x\\" || true || \\""')
  })
})

describe('refusing what cannot compile', () => {
  it.each([
    ['empty groups', { who: { kind: 'groups', groups: [] } }],
    ['blank role', { who: { kind: 'roles', roles: ['  '] } }],
    ['no users', { who: { kind: 'users', users: [] } }],
    ['no autonomous agents', { who: { kind: 'agents', clientIds: [] } }],
    ['no client ids', { agents: { kind: 'clients', clientIds: [] } }],
    ['no tools', { what: { kind: 'tools', tools: [] } }],
    ['a tool without a server', { what: { kind: 'tools', tools: ['list_users'] } }],
    ['a blank server', { what: { kind: 'server', server: '' } }],
    ['an empty hour window', { when: { weekdays: [], hours: { from: 8, to: 8 } } }],
    ['an hour out of range', { when: { weekdays: [], hours: { from: -1, to: 8 } } }],
    ['an end hour past 24', { when: { weekdays: [], hours: { from: 1, to: 25 } } }],
    ['a time condition with nothing in it', { when: { weekdays: [], hours: null } }],
    ['a weekday out of range', { when: { weekdays: [0], hours: null } }],
  ] as [string, Partial<RuleModel>][])('refuses %s', (_, over) => {
    const result = compileRule(rule(over))
    expect(result.ok).toBe(false)
    expect(ruleIssues(rule(over)).length).toBeGreaterThan(0)
  })

  it('reports nothing for a valid rule', () => {
    expect(ruleIssues(rule())).toEqual([])
  })

  it('trims identifiers', () => {
    const out = cedar({ who: { kind: 'groups', groups: ['  support '] } })
    expect(out).toContain('Group::"support"')
  })

  it('drops duplicates', () => {
    const out = cedar({ who: { kind: 'roles', roles: ['a', 'a'] } })
    expect(out).toContain('when { principal has roles && principal.roles.contains("a") }')
  })
})
