import type { RuleModel, What, When, Who } from './types'

export const NAMESPACE = 'FerrisKey'

export type CompileResult = { ok: true; cedar: string } | { ok: false; issues: string[] }

interface Parts {
  principal: string
  resource: string
  conditions: string[]
}

export function quote(value: string): string {
  let out = ''
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0
    if (char === '"') out += '\\"'
    else if (char === '\\') out += '\\\\'
    else if (code < 0x20 || code === 0x7f) out += `\\u{${code.toString(16)}}`
    else out += char
  }
  return `"${out}"`
}

function clean(values: string[]): string[] {
  const seen = new Set<string>()
  for (const value of values) {
    const trimmed = value.trim()
    if (trimmed.length > 0) seen.add(trimmed)
  }
  return [...seen]
}

function hasBlank(values: string[]): boolean {
  return values.some((value) => value.trim().length === 0)
}

function isHour(value: number, min: number, max: number): boolean {
  return Number.isInteger(value) && value >= min && value <= max
}

function listIssues(label: string, values: string[]): string[] {
  if (clean(values).length === 0) return [`Add at least one ${label}.`]
  if (hasBlank(values)) return [`Remove the blank ${label}.`]
  return []
}

function whoIssues(who: Who): string[] {
  switch (who.kind) {
    case 'any-user':
      return []
    case 'groups':
      return listIssues('group', who.groups)
    case 'roles':
      return listIssues('role', who.roles)
    case 'users':
      return listIssues('user', who.users)
    case 'agents':
      return listIssues('autonomous agent client id', who.clientIds)
  }
}

function whatIssues(what: What): string[] {
  switch (what.kind) {
    case 'tools': {
      const issues = listIssues('tool', what.tools)
      if (issues.length > 0) return issues
      return clean(what.tools).some((tool) => !/^[^/]+\/[^/]+$/.test(tool))
        ? ['Name each tool as server/tool.']
        : []
    }
    case 'server':
      return what.server.trim().length === 0 ? ['Pick a server.'] : []
    case 'read-only':
      return what.server !== null && what.server.trim().length === 0 ? ['Pick a server.'] : []
  }
}

function whenIssues(when: When | null): string[] {
  if (when === null) return []
  const issues: string[] = []
  if (when.weekdays.length === 0 && when.hours === null) {
    issues.push('Pick at least one weekday or an hour window.')
  }
  if (when.weekdays.some((day) => !isHour(day, 1, 7))) {
    issues.push('Weekdays run from 1 (Monday) to 7 (Sunday).')
  }
  if (when.hours !== null) {
    const { from, to } = when.hours
    if (!isHour(from, 0, 23) || !isHour(to, 1, 24)) {
      issues.push('Hours run from 0 to 23, and the window ends between 1 and 24.')
    } else if (from === to) {
      issues.push('The hour window is empty: start and end are the same.')
    }
  }
  return issues
}

export function ruleIssues(rule: RuleModel): string[] {
  const issues = [...whoIssues(rule.who)]
  if (rule.agents.kind === 'clients') issues.push(...listIssues('agent client id', rule.agents.clientIds))
  issues.push(...whatIssues(rule.what), ...whenIssues(rule.when))
  return issues
}

function entity(type: string, id: string): string {
  return `${NAMESPACE}::${type}::${quote(id)}`
}

function anyOf(parts: string[]): string {
  return parts.join(' || ')
}

function principalParts(who: Who): Pick<Parts, 'principal' | 'conditions'> {
  const user = `${NAMESPACE}::User`
  switch (who.kind) {
    case 'any-user':
      return { principal: `principal is ${user}`, conditions: [] }
    case 'groups': {
      const groups = clean(who.groups)
      if (groups.length === 1) {
        return { principal: `principal is ${user} in ${entity('Group', groups[0])}`, conditions: [] }
      }
      return {
        principal: `principal is ${user}`,
        conditions: [anyOf(groups.map((group) => `principal in ${entity('Group', group)}`))],
      }
    }
    case 'roles': {
      const tests = clean(who.roles).map((role) => `principal.roles.contains(${quote(role)})`)
      const test = tests.length === 1 ? tests[0] : `(${anyOf(tests)})`
      return { principal: `principal is ${user}`, conditions: [`principal has roles && ${test}`] }
    }
    case 'users': {
      const users = clean(who.users)
      if (users.length === 1) {
        return { principal: `principal == ${entity('User', users[0])}`, conditions: [] }
      }
      return {
        principal: `principal is ${user}`,
        conditions: [anyOf(users.map((id) => `principal == ${entity('User', id)}`))],
      }
    }
    case 'agents': {
      const agents = clean(who.clientIds)
      if (agents.length === 1) {
        return { principal: `principal == ${entity('Agent', agents[0])}`, conditions: [] }
      }
      return {
        principal: `principal is ${NAMESPACE}::Agent`,
        conditions: [anyOf(agents.map((id) => `principal == ${entity('Agent', id)}`))],
      }
    }
  }
}

function agentConditions(rule: RuleModel): string[] {
  if (rule.agents.kind === 'any') return []
  const ids = clean(rule.agents.clientIds)
  if (ids.length === 1) return [`context.agent == ${quote(ids[0])}`]
  return [`[${ids.map(quote).join(', ')}].contains(context.agent)`]
}

function resourceParts(what: What): Pick<Parts, 'resource' | 'conditions'> {
  const tool = `${NAMESPACE}::Tool`
  switch (what.kind) {
    case 'tools': {
      const tools = clean(what.tools)
      if (tools.length === 1) {
        return { resource: `resource == ${entity('Tool', tools[0])}`, conditions: [] }
      }
      return {
        resource: `resource is ${tool}`,
        conditions: [anyOf(tools.map((id) => `resource == ${entity('Tool', id)}`))],
      }
    }
    case 'server':
      return {
        resource: `resource is ${tool}`,
        conditions: [`resource.server == ${quote(what.server.trim())}`],
      }
    case 'read-only': {
      const conditions =
        what.server === null ? [] : [`resource.server == ${quote(what.server.trim())}`]
      conditions.push('resource has annotations && resource.annotations has readOnlyHint && resource.annotations.readOnlyHint')
      return { resource: `resource is ${tool}`, conditions }
    }
  }
}

function timeConditions(when: When | null): string[] {
  if (when === null) return []
  const conditions: string[] = []
  const days = [...new Set(when.weekdays)].sort((a, b) => a - b)
  if (days.length > 0 && days.length < 7) {
    conditions.push(`[${days.join(', ')}].contains(context.time.weekday)`)
  }
  if (when.hours !== null) {
    const { from, to } = when.hours
    const hour = 'context.time.hour'
    if (from === 0 && to === 24) return conditions
    if (from < to) {
      conditions.push(to === 24 ? `${hour} >= ${from}` : `${hour} >= ${from} && ${hour} < ${to}`)
    } else {
      conditions.push(`${hour} >= ${from} || ${hour} < ${to}`)
    }
  }
  return conditions
}

export function compileRule(rule: RuleModel): CompileResult {
  const issues = ruleIssues(rule)
  if (issues.length > 0) return { ok: false, issues }

  const who = principalParts(rule.who)
  const what = resourceParts(rule.what)
  const conditions = [
    ...who.conditions,
    ...agentConditions(rule),
    ...what.conditions,
    ...timeConditions(rule.when),
  ]

  const head = [
    `${rule.effect} (`,
    `  ${who.principal},`,
    `  action == ${NAMESPACE}::Action::"call_tool",`,
    `  ${what.resource}`,
    ')',
  ]
  const tail = conditions.map((condition) => `when { ${condition} }`)
  const lines = [...head, ...tail]
  lines[lines.length - 1] += ';'

  return { ok: true, cedar: lines.join('\n') }
}
