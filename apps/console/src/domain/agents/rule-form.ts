import type { Effect, RuleModel, What, When, Who } from './types'

export interface RuleForm {
  effect: Effect
  who: Who['kind']
  groups: string
  roles: string
  users: string
  autonomousIds: string
  agents: 'any' | 'clients'
  clientIds: string
  what: What['kind']
  tools: string[]
  readOnlyServer: string
  server: string
  limitDays: boolean
  weekdays: number[]
  limitHours: boolean
  from: number
  to: number
}

export function emptyForm(): RuleForm {
  return {
    effect: 'permit',
    who: 'any-user',
    groups: '',
    roles: '',
    users: '',
    autonomousIds: '',
    agents: 'any',
    clientIds: '',
    what: 'read-only',
    tools: [],
    readOnlyServer: '',
    server: '',
    limitDays: false,
    weekdays: [1, 2, 3, 4, 5],
    limitHours: false,
    from: 8,
    to: 18,
  }
}

export function splitList(text: string): string[] {
  const seen = new Set<string>()
  for (const part of text.split(/[\n,]/)) {
    const trimmed = part.trim()
    if (trimmed.length > 0) seen.add(trimmed)
  }
  return [...seen]
}

export function toggleItem<T>(items: T[], item: T): T[] {
  return items.includes(item) ? items.filter((existing) => existing !== item) : [...items, item]
}

function whoOf(form: RuleForm): Who {
  switch (form.who) {
    case 'any-user':
      return { kind: 'any-user' }
    case 'groups':
      return { kind: 'groups', groups: splitList(form.groups) }
    case 'roles':
      return { kind: 'roles', roles: splitList(form.roles) }
    case 'users':
      return { kind: 'users', users: splitList(form.users) }
    case 'agents':
      return { kind: 'agents', clientIds: splitList(form.autonomousIds) }
  }
}

function whatOf(form: RuleForm): What {
  switch (form.what) {
    case 'tools':
      return { kind: 'tools', tools: form.tools }
    case 'server':
      return { kind: 'server', server: form.server }
    case 'read-only':
      return { kind: 'read-only', server: form.readOnlyServer === '' ? null : form.readOnlyServer }
  }
}

function whenOf(form: RuleForm): When | null {
  if (!form.limitDays && !form.limitHours) return null
  return {
    weekdays: form.limitDays ? [...new Set(form.weekdays)].sort((a, b) => a - b) : [],
    hours: form.limitHours ? { from: form.from, to: form.to } : null,
  }
}

export function formToRule(form: RuleForm): RuleModel {
  return {
    version: 1,
    effect: form.effect,
    who: whoOf(form),
    agents:
      form.agents === 'any'
        ? { kind: 'any' }
        : { kind: 'clients', clientIds: splitList(form.clientIds) },
    what: whatOf(form),
    when: whenOf(form),
  }
}

export function ruleToForm(rule: RuleModel): RuleForm {
  const form: RuleForm = { ...emptyForm(), effect: rule.effect, who: rule.who.kind, what: rule.what.kind }

  switch (rule.who.kind) {
    case 'groups':
      form.groups = rule.who.groups.join('\n')
      break
    case 'roles':
      form.roles = rule.who.roles.join('\n')
      break
    case 'users':
      form.users = rule.who.users.join('\n')
      break
    case 'agents':
      form.autonomousIds = rule.who.clientIds.join('\n')
      break
    case 'any-user':
      break
  }

  if (rule.agents.kind === 'clients') {
    form.agents = 'clients'
    form.clientIds = rule.agents.clientIds.join('\n')
  }

  switch (rule.what.kind) {
    case 'tools':
      form.tools = rule.what.tools
      break
    case 'server':
      form.server = rule.what.server
      break
    case 'read-only':
      form.readOnlyServer = rule.what.server ?? ''
      break
  }

  if (rule.when !== null) {
    form.limitDays = rule.when.weekdays.length > 0
    form.weekdays = rule.when.weekdays.length > 0 ? rule.when.weekdays : form.weekdays
    form.limitHours = rule.when.hours !== null
    if (rule.when.hours !== null) {
      form.from = rule.when.hours.from
      form.to = rule.when.hours.to
    }
  }

  return form
}
