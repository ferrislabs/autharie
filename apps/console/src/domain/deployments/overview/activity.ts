import type { Schemas } from '@/api/api.client'
import type { ActivityEntry, ActivityOutcome } from './activity-feed'

const SHOWN = 8

export function outcomeOf(status: Schemas.ActionStatus): ActivityOutcome {
  if (status === 'Pending') return 'running'
  if ('Leased' in status || 'Pulled' in status) return 'running'
  if ('Published' in status) return 'done'
  return 'failed'
}

function field(action: Schemas.Action, key: string): string | undefined {
  const data = action.payload.data
  if (typeof data !== 'object' || data === null) return undefined
  const value = (data as Record<string, unknown>)[key]
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

function detailOf(action: Schemas.Action): string | undefined {
  const from = field(action, 'from_version')
  const to = field(action, 'to_version')
  return from && to ? `${from} → ${to}` : undefined
}

function createdAt(action: Schemas.Action): number {
  return Date.parse(action.metadata.created_at)
}

function newestFirst(actions: Schemas.Action[]): Schemas.Action[] {
  return [...actions].sort((a, b) => createdAt(b) - createdAt(a))
}

export function toActivity(actions: Schemas.Action[]): ActivityEntry[] {
  return newestFirst(actions)
    .filter((action) => !action.action_type.startsWith('system.'))
    .slice(0, SHOWN)
    .map((action) => ({
      id: action.id,
      type: action.action_type,
      at: createdAt(action),
      outcome: outcomeOf(action.status),
      detail: detailOf(action),
    }))
}

