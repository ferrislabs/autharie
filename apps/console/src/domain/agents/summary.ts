import type { AgentPolicy, Effect, HourWindow, RuleModel, What, Who } from './types'

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

export function summariseWeekdays(weekdays: number[]): string {
  const days = [...new Set(weekdays)].sort((a, b) => a - b)
  if (days.length === 7) return 'every day'

  const parts: string[] = []
  let index = 0
  while (index < days.length) {
    let end = index
    while (end + 1 < days.length && days[end + 1] === days[end] + 1) end += 1
    if (end - index >= 2) {
      parts.push(`${DAYS[days[index] - 1]}–${DAYS[days[end] - 1]}`)
    } else {
      for (let i = index; i <= end; i += 1) parts.push(DAYS[days[i] - 1])
    }
    index = end + 1
  }
  return parts.join(', ')
}

function pad(hour: number): string {
  return String(hour).padStart(2, '0')
}

export function summariseHours(hours: HourWindow): string {
  return `${pad(hours.from)}:00–${pad(hours.to)}:00 UTC`
}

function plural(count: number, one: string, many: string): string {
  return count === 1 ? one : many
}

function describeWho(who: Who): string {
  switch (who.kind) {
    case 'any-user':
      return 'any user'
    case 'groups':
      return `users in ${plural(who.groups.length, 'group', 'groups')} ${who.groups.join(', ')}`
    case 'roles':
      return `users with ${plural(who.roles.length, 'role', 'roles')} ${who.roles.join(', ')}`
    case 'users':
      return `${plural(who.users.length, 'user', 'users')} ${who.users.join(', ')}`
    case 'agents':
      return `${plural(who.clientIds.length, 'autonomous agent', 'autonomous agents')} ${who.clientIds.join(', ')}`
  }
}

function describeWhat(what: What): string {
  switch (what.kind) {
    case 'tools':
      return what.tools.length === 1 ? what.tools[0] : `${what.tools.length} tools`
    case 'server':
      return `all ${what.server} tools`
    case 'read-only':
      return what.server === null ? 'read-only tools' : `read-only ${what.server} tools`
  }
}

export function describeRule(rule: RuleModel): string {
  const verb = rule.effect === 'permit' ? 'Permit' : 'Forbid'
  let line = `${verb} ${describeWho(rule.who)} to call ${describeWhat(rule.what)}`

  if (rule.agents.kind === 'clients') line += ` through ${rule.agents.clientIds.join(', ')}`

  if (rule.when !== null) {
    if (rule.when.weekdays.length > 0) line += `, ${summariseWeekdays(rule.when.weekdays)}`
    if (rule.when.hours !== null) line += `, ${summariseHours(rule.when.hours)}`
  }

  return line
}

export function effectOfCedar(cedar: string): Effect | null {
  const match = /(?:^|[\s;}])(permit|forbid)\s*\(/.exec(cedar)
  return match ? (match[1] as Effect) : null
}

export function summarisePolicy(policy: AgentPolicy): string {
  if (policy.rule) return describeRule(policy.rule)
  const effect = effectOfCedar(policy.cedar)
  if (effect === null) return 'Custom Cedar'
  return `${effect === 'permit' ? 'Permit' : 'Forbid'} · custom Cedar`
}
