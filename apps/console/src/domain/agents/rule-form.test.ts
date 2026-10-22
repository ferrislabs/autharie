import { describe, expect, it } from 'vitest'
import { compileRule } from './compile-rule'
import { emptyForm, formToRule, ruleToForm, splitList, toggleItem } from './rule-form'

describe('splitting a typed list', () => {
  it('splits on lines and commas and trims', () => {
    expect(splitList(' a, b\nc ,, \n d')).toEqual(['a', 'b', 'c', 'd'])
  })

  it('drops duplicates and keeps order', () => {
    expect(splitList('b\na\nb')).toEqual(['b', 'a'])
  })

  it('gives nothing for blank text', () => {
    expect(splitList('  \n ,')).toEqual([])
  })
})

describe('toggling an item', () => {
  it('adds a missing item', () => {
    expect(toggleItem(['a'], 'b')).toEqual(['a', 'b'])
  })

  it('removes a present item', () => {
    expect(toggleItem(['a', 'b'], 'a')).toEqual(['b'])
  })
})

describe('the empty form', () => {
  it('reads as: permit any user and any agent to call read-only tools, at any time', () => {
    expect(formToRule(emptyForm())).toEqual({
      version: 1,
      effect: 'permit',
      who: { kind: 'any-user' },
      agents: { kind: 'any' },
      what: { kind: 'read-only', server: null },
      when: null,
    })
  })
})

describe('form to rule', () => {
  it('reads groups from the typed text', () => {
    const rule = formToRule({ ...emptyForm(), who: 'groups', groups: 'support\nops' })
    expect(rule.who).toEqual({ kind: 'groups', groups: ['support', 'ops'] })
  })

  it('reads roles, users and autonomous agents', () => {
    expect(formToRule({ ...emptyForm(), who: 'roles', roles: 'admin' }).who).toEqual({
      kind: 'roles',
      roles: ['admin'],
    })
    expect(formToRule({ ...emptyForm(), who: 'users', users: 'u-1, u-2' }).who).toEqual({
      kind: 'users',
      users: ['u-1', 'u-2'],
    })
    expect(formToRule({ ...emptyForm(), who: 'agents', autonomousIds: 'bot' }).who).toEqual({
      kind: 'agents',
      clientIds: ['bot'],
    })
  })

  it('keeps the text of the other kinds out of the rule', () => {
    const rule = formToRule({ ...emptyForm(), who: 'any-user', groups: 'support' })
    expect(rule.who).toEqual({ kind: 'any-user' })
  })

  it('reads specific agents', () => {
    const rule = formToRule({ ...emptyForm(), agents: 'clients', clientIds: 'a\nb' })
    expect(rule.agents).toEqual({ kind: 'clients', clientIds: ['a', 'b'] })
  })

  it('reads the chosen tools', () => {
    const rule = formToRule({ ...emptyForm(), what: 'tools', tools: ['autharie/list_backups'] })
    expect(rule.what).toEqual({ kind: 'tools', tools: ['autharie/list_backups'] })
  })

  it('reads read-only tools of one server', () => {
    const rule = formToRule({ ...emptyForm(), what: 'read-only', readOnlyServer: 'ferriskey' })
    expect(rule.what).toEqual({ kind: 'read-only', server: 'ferriskey' })
  })

  it('reads all tools of a server', () => {
    const rule = formToRule({ ...emptyForm(), what: 'server', server: 'autharie' })
    expect(rule.what).toEqual({ kind: 'server', server: 'autharie' })
  })

  it('reads no time condition unless one is switched on', () => {
    expect(formToRule({ ...emptyForm(), weekdays: [1], from: 9, to: 10 }).when).toBeNull()
  })

  it('reads weekdays alone', () => {
    const rule = formToRule({ ...emptyForm(), limitDays: true, weekdays: [1, 2] })
    expect(rule.when).toEqual({ weekdays: [1, 2], hours: null })
  })

  it('reads hours alone', () => {
    const rule = formToRule({ ...emptyForm(), limitHours: true, from: 8, to: 18 })
    expect(rule.when).toEqual({ weekdays: [], hours: { from: 8, to: 18 } })
  })

  it('reads weekdays and hours together', () => {
    const rule = formToRule({
      ...emptyForm(),
      limitDays: true,
      weekdays: [1, 2, 3, 4, 5],
      limitHours: true,
      from: 8,
      to: 18,
    })
    expect(rule.when).toEqual({ weekdays: [1, 2, 3, 4, 5], hours: { from: 8, to: 18 } })
  })

  it('keeps an empty weekday choice so the compiler can refuse it', () => {
    const rule = formToRule({ ...emptyForm(), limitDays: true, weekdays: [] })
    expect(rule.when).toEqual({ weekdays: [], hours: null })
  })

  it('sorts the weekdays', () => {
    const rule = formToRule({ ...emptyForm(), limitDays: true, weekdays: [5, 1] })
    expect(rule.when?.weekdays).toEqual([1, 5])
  })

  it('carries the effect', () => {
    expect(formToRule({ ...emptyForm(), effect: 'forbid' }).effect).toBe('forbid')
  })
})

describe('rule to form and back', () => {
  const forms = [
    emptyForm(),
    { ...emptyForm(), effect: 'forbid' as const, who: 'groups' as const, groups: 'a\nb' },
    { ...emptyForm(), who: 'roles' as const, roles: 'x' },
    { ...emptyForm(), who: 'users' as const, users: 'u' },
    { ...emptyForm(), who: 'agents' as const, autonomousIds: 'bot' },
    { ...emptyForm(), agents: 'clients' as const, clientIds: 'a' },
    { ...emptyForm(), what: 'tools' as const, tools: ['autharie/list_backups'] },
    { ...emptyForm(), what: 'read-only' as const, readOnlyServer: 'autharie' },
    { ...emptyForm(), what: 'server' as const, server: 'ferriskey' },
    { ...emptyForm(), limitDays: true, weekdays: [1, 2, 3, 4, 5], limitHours: true, from: 22, to: 6 },
  ]

  it.each(forms.map((form, index) => [index, form] as const))('round-trips form %i', (_, form) => {
    const rule = formToRule(form)
    expect(formToRule(ruleToForm(rule))).toEqual(rule)
    expect(compileRule(rule).ok).toBe(true)
  })
})
