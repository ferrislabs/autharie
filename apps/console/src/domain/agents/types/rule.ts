export type Effect = 'permit' | 'forbid'

export type Who =
  | { kind: 'any-user' }
  | { kind: 'groups'; groups: string[] }
  | { kind: 'roles'; roles: string[] }
  | { kind: 'users'; users: string[] }
  | { kind: 'agents'; clientIds: string[] }

export type WhichAgents = { kind: 'any' } | { kind: 'clients'; clientIds: string[] }

export type What =
  | { kind: 'tools'; tools: string[] }
  | { kind: 'read-only'; server: string | null }
  | { kind: 'server'; server: string }

export interface HourWindow {
  from: number
  to: number
}

export interface When {
  weekdays: number[]
  hours: HourWindow | null
}

export interface RuleModel {
  version: 1
  effect: Effect
  who: Who
  agents: WhichAgents
  what: What
  when: When | null
}
