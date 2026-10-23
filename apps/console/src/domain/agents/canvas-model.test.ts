import { describe, expect, it } from 'vitest'
import {
  addBox,
  addLink,
  addTemplate,
  canLink,
  changeCount,
  COLUMN_X,
  connectedRules,
  diffDoc,
  docFromPolicies,
  duplicateRule,
  keepPositions,
  moveBoxes,
  newRuleBox,
  newWhatBox,
  newWhoBox,
  pendingChanges,
  removeBox,
  removeLink,
  ruleFromTemplate,
  ruleOf,
  sameRule,
  updateBox,
  type CanvasDoc,
  type RuleBox,
  type WhatBox,
  type WhoBox,
} from './canvas-model'
import { compileRule } from './compile-rule'
import { POLICY_TEMPLATES } from './policy-templates'
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

function policy(id: string, model: RuleModel | undefined, over: Partial<AgentPolicy> = {}): AgentPolicy {
  const compiled = model ? compileRule(model) : null
  return {
    id,
    name: `Policy ${id}`,
    enabled: true,
    cedar: compiled && compiled.ok ? compiled.cedar : 'permit (principal, action, resource);',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...(model ? { rule: model } : {}),
    ...over,
  }
}

function boxesOf<K extends 'who' | 'rule' | 'what'>(doc: CanvasDoc, kind: K) {
  return doc.boxes.filter((box) => box.kind === kind) as K extends 'who'
    ? WhoBox[]
    : K extends 'rule'
      ? RuleBox[]
      : WhatBox[]
}

function ok(doc: CanvasDoc, id: string): RuleModel {
  const result = ruleOf(doc, id)
  if (!result.ok) throw new Error(result.issues.join(' | '))
  return result.rule
}

function issues(doc: CanvasDoc, id: string): string[] {
  const result = ruleOf(doc, id)
  if (result.ok) throw new Error('expected issues')
  return result.issues
}

function blank(): { doc: CanvasDoc; ruleId: string } {
  const box = newRuleBox({ boxes: [], links: [], positions: {} })
  return { doc: addBox({ boxes: [], links: [], positions: {} }, box), ruleId: box.id }
}

function withWho(doc: CanvasDoc, ruleId: string, subject: WhoBox['subject']) {
  const box = newWhoBox(doc, subject)
  return { doc: addLink(addBox(doc, box), box.id, ruleId), id: box.id }
}

function withWhat(doc: CanvasDoc, ruleId: string, target: WhatBox['target']) {
  const box = newWhatBox(doc, target)
  return { doc: addLink(addBox(doc, box), ruleId, box.id), id: box.id }
}

describe('docFromPolicies', () => {
  it('builds one rule box per policy with a rule', () => {
    const doc = docFromPolicies([
      policy('a', rule()),
      policy('b', rule({ effect: 'forbid' })),
      policy('raw', undefined),
    ])
    expect(boxesOf(doc, 'rule').map((box) => box.id)).toEqual(['rule:a', 'rule:b'])
    expect(boxesOf(doc, 'rule')[1].effect).toBe('forbid')
    expect(boxesOf(doc, 'rule')[0].policyId).toBe('a')
    expect(doc.boxes.some((box) => box.id.includes('raw'))).toBe(false)
  })

  it('shares who boxes between rules that name the same subject', () => {
    const doc = docFromPolicies([
      policy('a', rule({ who: { kind: 'roles', roles: ['admin'] } })),
      policy('b', rule({ who: { kind: 'roles', roles: ['admin', 'ops'] } })),
    ])
    const whos = boxesOf(doc, 'who')
    expect(whos.map((box) => box.subject)).toEqual([
      { type: 'role', value: 'admin' },
      { type: 'role', value: 'ops' },
    ])
    expect(doc.links.filter((link) => link.source === whos[0].id)).toHaveLength(2)
  })

  it('shares what boxes by target and ignores the order of a tools list', () => {
    const doc = docFromPolicies([
      policy('a', rule({ what: { kind: 'tools', tools: ['ferriskey/b', 'ferriskey/a'] } })),
      policy('b', rule({ what: { kind: 'tools', tools: ['ferriskey/a', 'ferriskey/b'] } })),
      policy('c', rule({ what: { kind: 'read-only', server: null } })),
      policy('d', rule({ what: { kind: 'read-only', server: null } })),
      policy('e', rule({ what: { kind: 'server', server: 'ferriskey' } })),
    ])
    expect(boxesOf(doc, 'what')).toHaveLength(3)
  })

  it('maps autonomous agents to agent subjects', () => {
    const doc = docFromPolicies([policy('a', rule({ who: { kind: 'agents', clientIds: ['bot'] } }))])
    expect(boxesOf(doc, 'who')[0].subject).toEqual({ type: 'agent', value: 'bot' })
  })

  it('lays boxes out in three columns without overlap', () => {
    const doc = docFromPolicies([
      policy('a', rule({ who: { kind: 'groups', groups: ['x', 'y', 'z'] } })),
      policy('b', rule({ who: { kind: 'roles', roles: ['r'] } })),
      policy('c', rule({ who: { kind: 'users', users: ['u1', 'u2'] } })),
    ])
    for (const box of doc.boxes) expect(doc.positions[box.id]).toBeDefined()
    for (const kind of ['who', 'rule', 'what'] as const) {
      const ys = boxesOf(doc, kind)
        .map((box) => doc.positions[box.id])
        .map((position) => {
          expect(position.x).toBe(COLUMN_X[kind])
          return position.y
        })
        .sort((a, b) => a - b)
      for (let i = 1; i < ys.length; i += 1) expect(ys[i] - ys[i - 1]).toBeGreaterThanOrEqual(80)
    }
  })

  it('is deterministic', () => {
    const policies = [policy('a', rule()), policy('b', rule({ effect: 'forbid' }))]
    expect(docFromPolicies(policies)).toEqual(docFromPolicies(policies))
  })
})

describe('ruleOf round trip', () => {
  for (const template of POLICY_TEMPLATES) {
    it(`gives back the ${template.id} template`, () => {
      const built = template.build()
      const doc = docFromPolicies([policy('t', built.rule)])
      const back = ok(doc, 'rule:t')
      expect(back).toEqual(built.rule)
      expect(compileRule(back).ok).toBe(true)
    })
  }

  const multi: [string, RuleModel][] = [
    ['groups', rule({ who: { kind: 'groups', groups: ['eng', 'ops'] } })],
    ['roles', rule({ who: { kind: 'roles', roles: ['a', 'b', 'c'] } })],
    ['users', rule({ who: { kind: 'users', users: ['u1', 'u2'] } })],
    ['agents', rule({ who: { kind: 'agents', clientIds: ['bot1', 'bot2'] } })],
    ['clients', rule({ agents: { kind: 'clients', clientIds: ['cursor', 'claude'] } })],
    ['tools', rule({ what: { kind: 'tools', tools: ['ferriskey/get_user', 'autharie/list_backups'] } })],
    ['server', rule({ effect: 'forbid', what: { kind: 'server', server: 'autharie' } })],
    ['read-only server', rule({ what: { kind: 'read-only', server: 'ferriskey' } })],
    ['when', rule({ when: { weekdays: [6, 7], hours: { from: 22, to: 6 } } })],
  ]
  for (const [label, model] of multi) {
    it(`keeps a multi-value ${label} rule`, () => {
      const doc = docFromPolicies([policy('m', model)])
      const back = ok(doc, 'rule:m')
      expect(sameRule(back, model)).toBe(true)
      expect(compileRule(back).ok).toBe(true)
    })
  }

  it('keeps the rule when its boxes are shared with another rule', () => {
    const a = rule({ who: { kind: 'roles', roles: ['admin'] } })
    const b = rule({ who: { kind: 'roles', roles: ['admin', 'ops'] }, effect: 'forbid' })
    const doc = docFromPolicies([policy('a', a), policy('b', b)])
    expect(ok(doc, 'rule:a')).toEqual(a)
    expect(ok(doc, 'rule:b')).toEqual(b)
  })

  it('carries the box settings into the rule', () => {
    const { doc, ruleId } = blank()
    const box = doc.boxes[0] as RuleBox
    let next = updateBox(doc, {
      ...box,
      effect: 'forbid',
      agents: { kind: 'clients', clientIds: ['cursor'] },
      when: { weekdays: [1], hours: null },
    })
    next = withWho(next, ruleId, { type: 'any' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: null }).doc
    expect(ok(next, ruleId)).toEqual(
      rule({
        effect: 'forbid',
        agents: { kind: 'clients', clientIds: ['cursor'] },
        when: { weekdays: [1], hours: null },
      }),
    )
  })
})

describe('ruleOf issues', () => {
  it('asks for a subject', () => {
    const { doc, ruleId } = blank()
    const next = withWhat(doc, ruleId, { type: 'read-only', server: null }).doc
    expect(issues(next, ruleId)).toEqual([expect.stringMatching(/who|people|person/i)])
  })

  it('asks for a target', () => {
    const { doc, ruleId } = blank()
    const next = withWho(doc, ruleId, { type: 'any' }).doc
    expect(issues(next, ruleId)).toEqual([expect.stringMatching(/tool/i)])
  })

  it('lists both when nothing is linked', () => {
    const { doc, ruleId } = blank()
    expect(issues(doc, ruleId)).toHaveLength(2)
  })

  it('refuses a mix of subject types', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'group', value: 'eng' }).doc
    next = withWho(next, ruleId, { type: 'role', value: 'admin' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: null }).doc
    expect(issues(next, ruleId)).toContain(
      'A rule applies to groups, roles, users or agents — not a mix. Split it into two rules.',
    )
  })

  it('refuses any user next to other subjects', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'any' }).doc
    next = withWho(next, ruleId, { type: 'group', value: 'eng' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: null }).doc
    const found = issues(next, ruleId)
    expect(found).toHaveLength(1)
    expect(found[0]).toMatch(/any user/i)
  })

  it('merges several tools boxes into one list', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'any' }).doc
    next = withWhat(next, ruleId, { type: 'tools', tools: ['ferriskey/get_user'] }).doc
    next = withWhat(next, ruleId, { type: 'tools', tools: ['ferriskey/get_user', 'autharie/list_backups'] }).doc
    expect(ok(next, ruleId).what).toEqual({
      kind: 'tools',
      tools: ['ferriskey/get_user', 'autharie/list_backups'],
    })
  })

  it('refuses read-only next to another target', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'any' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: null }).doc
    next = withWhat(next, ruleId, { type: 'tools', tools: ['ferriskey/get_user'] }).doc
    expect(issues(next, ruleId)).toEqual([expect.stringMatching(/read-only|server/i)])
  })

  it('refuses two server targets', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'any' }).doc
    next = withWhat(next, ruleId, { type: 'server', server: 'ferriskey' }).doc
    next = withWhat(next, ruleId, { type: 'server', server: 'autharie' }).doc
    expect(issues(next, ruleId)).toHaveLength(1)
  })

  it('adds the compile issues', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'group', value: '' }).doc
    next = withWhat(next, ruleId, { type: 'tools', tools: [] }).doc
    expect(issues(next, ruleId)).toEqual(['Add at least one group.', 'Add at least one tool.'])
  })

  it('asks for a name', () => {
    const { doc, ruleId } = blank()
    const box = doc.boxes[0] as RuleBox
    let next = updateBox(doc, { ...box, name: '  ' })
    next = withWho(next, ruleId, { type: 'any' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: null }).doc
    expect(issues(next, ruleId)).toEqual(['Give the rule a name.'])
  })

  it('reports an unknown rule', () => {
    expect(ruleOf({ boxes: [], links: [], positions: {} }, 'rule:none')).toEqual({
      ok: false,
      issues: ['This rule no longer exists.'],
    })
  })
})

describe('reducers', () => {
  it('only links who to rule and rule to what', () => {
    const { doc, ruleId } = blank()
    const who = newWhoBox(doc, { type: 'any' })
    const what = newWhatBox(doc, { type: 'read-only', server: null })
    const next = addBox(addBox(doc, who), what)
    expect(canLink(next, who.id, ruleId)).toBe(true)
    expect(canLink(next, ruleId, what.id)).toBe(true)
    expect(canLink(next, who.id, what.id)).toBe(false)
    expect(canLink(next, ruleId, who.id)).toBe(false)
    expect(canLink(next, what.id, ruleId)).toBe(false)
    expect(canLink(next, who.id, 'ghost')).toBe(false)
    expect(addLink(next, who.id, what.id)).toBe(next)
  })

  it('rejects a duplicate link', () => {
    const { doc, ruleId } = blank()
    const { doc: linked, id } = withWho(doc, ruleId, { type: 'any' })
    expect(canLink(linked, id, ruleId)).toBe(false)
    expect(addLink(linked, id, ruleId)).toBe(linked)
  })

  it('gives every new box a fresh id', () => {
    let doc: CanvasDoc = { boxes: [], links: [], positions: {} }
    const ids = new Set<string>()
    for (let i = 0; i < 4; i += 1) {
      const box = newWhoBox(doc, { type: 'any' })
      ids.add(box.id)
      doc = addBox(doc, box)
    }
    expect(ids.size).toBe(4)
  })

  it('removes a box with its links', () => {
    const { doc, ruleId } = blank()
    const first = withWho(doc, ruleId, { type: 'any' })
    const second = withWhat(first.doc, ruleId, { type: 'read-only', server: null })
    const removed = removeBox(second.doc, first.id)
    expect(removed.boxes.some((box) => box.id === first.id)).toBe(false)
    expect(removed.links).toHaveLength(1)
    expect(removed.positions[first.id]).toBeUndefined()
    expect(removeBox(second.doc, ruleId).links).toEqual([])
  })

  it('removes one link', () => {
    const { doc, ruleId } = blank()
    const first = withWho(doc, ruleId, { type: 'any' })
    const id = first.doc.links[0].id
    expect(removeLink(first.doc, id).links).toEqual([])
    expect(removeLink(first.doc, id).boxes).toHaveLength(2)
  })

  it('updates a box in place', () => {
    const { doc, ruleId } = blank()
    const first = withWho(doc, ruleId, { type: 'group', value: '' })
    const next = updateBox(first.doc, { id: first.id, kind: 'who', subject: { type: 'group', value: 'eng' } })
    expect((next.boxes.find((box) => box.id === first.id) as WhoBox).subject).toEqual({
      type: 'group',
      value: 'eng',
    })
    expect(next.links).toEqual(first.doc.links)
  })

  it('moves boxes', () => {
    const { doc, ruleId } = blank()
    expect(moveBoxes(doc, { [ruleId]: { x: 5, y: 6 } }).positions[ruleId]).toEqual({ x: 5, y: 6 })
  })

  it('duplicates a rule as a new unpublished one sharing its boxes', () => {
    const doc = docFromPolicies([policy('a', rule({ who: { kind: 'roles', roles: ['admin'] } }))])
    const next = duplicateRule(doc, 'rule:a')
    const rules = boxesOf(next, 'rule')
    expect(rules).toHaveLength(2)
    const copy = rules[1]
    expect(copy.policyId).toBeNull()
    expect(copy.name).not.toBe(rules[0].name)
    expect(ok(next, copy.id)).toEqual(ok(next, 'rule:a'))
    expect(boxesOf(next, 'who')).toHaveLength(1)
    expect(next.positions[copy.id]).toBeDefined()
  })

  it('keeps positions of boxes that survive a rebuild', () => {
    const before = moveBoxes(docFromPolicies([policy('a', rule())]), { 'rule:a': { x: 9, y: 9 } })
    const after = keepPositions(docFromPolicies([policy('a', rule()), policy('b', rule())]), before)
    expect(after.positions['rule:a']).toEqual({ x: 9, y: 9 })
    expect(after.positions['rule:b']).toBeDefined()
  })

  it('finds the rules around a box', () => {
    const doc = docFromPolicies([
      policy('a', rule({ who: { kind: 'roles', roles: ['admin'] } })),
      policy('b', rule({ who: { kind: 'roles', roles: ['admin'] }, what: { kind: 'server', server: 'ferriskey' } })),
      policy('c', rule({ who: { kind: 'roles', roles: ['ops'] } })),
    ])
    expect(connectedRules(doc, 'who:role:admin').sort()).toEqual(['rule:a', 'rule:b'])
    expect(connectedRules(doc, 'rule:c')).toEqual(['rule:c'])
    expect(connectedRules(doc, 'what:server:ferriskey')).toEqual(['rule:b'])
    expect(connectedRules(doc, 'ghost')).toEqual([])
  })
})

describe('templates', () => {
  for (const template of POLICY_TEMPLATES) {
    it(`drops ${template.id} as a compilable rule`, () => {
      const doc = addTemplate({ boxes: [], links: [], positions: {} }, template, { x: 0, y: 0 })
      const rules = boxesOf(doc, 'rule')
      expect(rules).toHaveLength(1)
      expect(rules[0].policyId).toBeNull()
      expect(rules[0].name).toBe(template.build().name)
      expect(ok(doc, rules[0].id)).toEqual(template.build().rule)
      for (const box of doc.boxes) expect(doc.positions[box.id]).toBeDefined()
    })
  }

  it('does not clash with boxes already on the canvas', () => {
    const template = POLICY_TEMPLATES[0]
    let doc = addTemplate({ boxes: [], links: [], positions: {} }, template, { x: 0, y: 0 })
    doc = addTemplate(doc, template, { x: 0, y: 300 })
    expect(new Set(doc.boxes.map((box) => box.id)).size).toBe(doc.boxes.length)
    expect(boxesOf(doc, 'rule')).toHaveLength(2)
  })

  it('returns boxes and links', () => {
    const dropped = ruleFromTemplate(POLICY_TEMPLATES[2])
    expect(dropped.boxes).toHaveLength(3)
    expect(dropped.links).toHaveLength(2)
    expect(dropped.boxes.some((box) => box.id === dropped.ruleId)).toBe(true)
  })
})

describe('diffDoc', () => {
  const a = policy('a', rule({ who: { kind: 'roles', roles: ['admin'] } }), { description: 'desc' })
  const b = policy('b', rule({ effect: 'forbid', what: { kind: 'server', server: 'ferriskey' } }))
  const raw = policy('raw', undefined)

  it('produces nothing for an untouched canvas', () => {
    const diff = diffDoc([a, b, raw], docFromPolicies([a, b, raw]))
    expect(diff).toEqual({ create: [], createIds: [], update: [], delete: [] })
    expect(changeCount(diff)).toBe(0)
  })

  it('ignores moves and the order of lists', () => {
    const tools = policy('t', rule({ what: { kind: 'tools', tools: ['ferriskey/b', 'ferriskey/a'] } }))
    const doc = moveBoxes(docFromPolicies([tools]), { 'rule:t': { x: 1, y: 1 } })
    expect(changeCount(diffDoc([tools], doc))).toBe(0)
  })

  it('creates a new rule with its cedar', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'group', value: 'eng' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: 'ferriskey' }).doc
    const diff = diffDoc([], next)
    expect(diff.create).toHaveLength(1)
    expect(diff.createIds).toEqual([ruleId])
    const input = diff.create[0]
    expect(input.name).toBe('New rule')
    expect(input.enabled).toBe(true)
    expect(input.description).toBeUndefined()
    expect(input.rule).toEqual(ok(next, ruleId))
    expect(input.cedar).toContain('permit')
    expect(changeCount(diff)).toBe(1)
  })

  it('does not create a rule that still has issues', () => {
    const { doc } = blank()
    const diff = diffDoc([], doc)
    expect(diff.create).toEqual([])
    const pending = pendingChanges([], doc)
    expect(pending.blocking).toHaveLength(1)
    expect(pending.blocking[0].issues.length).toBeGreaterThan(0)
    expect(pending.count).toBe(1)
  })

  it('updates a renamed rule', () => {
    const doc = docFromPolicies([a, b])
    const box = doc.boxes.find((entry) => entry.id === 'rule:a') as RuleBox
    const diff = diffDoc([a, b], updateBox(doc, { ...box, name: 'Renamed', enabled: false }))
    expect(diff.update).toHaveLength(1)
    expect(diff.update[0].id).toBe('a')
    expect(diff.update[0].input).toMatchObject({ name: 'Renamed', enabled: false, description: 'desc' })
    expect(diff.update[0].input.rule).toEqual(a.rule)
    expect(diff.create).toEqual([])
    expect(diff.delete).toEqual([])
  })

  it('updates a rule when a link changes', () => {
    const doc = docFromPolicies([a])
    const extra = newWhoBox(doc, { type: 'role', value: 'ops' })
    const next = addLink(addBox(doc, extra), extra.id, 'rule:a')
    const diff = diffDoc([a], next)
    expect(diff.update).toHaveLength(1)
    expect(diff.update[0].input.rule?.who).toEqual({ kind: 'roles', roles: ['admin', 'ops'] })
  })

  it('updates every rule that shares an edited box', () => {
    const one = policy('one', rule({ who: { kind: 'roles', roles: ['admin'] } }))
    const two = policy('two', rule({ who: { kind: 'roles', roles: ['admin'] }, effect: 'forbid' }))
    const doc = docFromPolicies([one, two])
    const next = updateBox(doc, { id: 'who:role:admin', kind: 'who', subject: { type: 'role', value: 'root' } })
    expect(diffDoc([one, two], next).update.map((entry) => entry.id).sort()).toEqual(['one', 'two'])
  })

  it('deletes a removed rule and leaves raw policies alone', () => {
    const doc = removeBox(docFromPolicies([a, b, raw]), 'rule:b')
    const diff = diffDoc([a, b, raw], doc)
    expect(diff.delete).toEqual(['b'])
    expect(changeCount(diff)).toBe(1)
  })

  it('removing a new rule leaves no trace', () => {
    const { doc, ruleId } = blank()
    expect(changeCount(diffDoc([], removeBox(doc, ruleId)))).toBe(0)
  })

  it('counts creates, updates and deletes together', () => {
    let doc = docFromPolicies([a, b])
    doc = removeBox(doc, 'rule:b')
    const box = doc.boxes.find((entry) => entry.id === 'rule:a') as RuleBox
    doc = updateBox(doc, { ...box, name: 'x' })
    const added = addTemplate(doc, POLICY_TEMPLATES[0], { x: 0, y: 500 })
    expect(changeCount(diffDoc([a, b], added))).toBe(3)
    expect(pendingChanges([a, b], added).changedRuleIds.size).toBe(2)
  })

  it('treats a rule published during this session as unchanged once the server agrees', () => {
    const { doc, ruleId } = blank()
    let next = withWho(doc, ruleId, { type: 'any' }).doc
    next = withWhat(next, ruleId, { type: 'read-only', server: null }).doc
    const box = next.boxes.find((entry) => entry.id === ruleId) as RuleBox
    const published = updateBox(next, { ...box, policyId: 'p1' })
    const server = policy('p1', rule(), { name: 'New rule' })
    expect(changeCount(diffDoc([server], published))).toBe(0)
    expect(changeCount(diffDoc([], published))).toBe(0)
  })
})
