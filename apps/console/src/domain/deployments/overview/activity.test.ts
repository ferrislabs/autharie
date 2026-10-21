import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { hostnameFromActions, outcomeOf, toActivity } from './activity'

function action(overrides: Partial<Schemas.Action> & { id: string }): Schemas.Action {
  return {
    action_type: 'deployment.backup',
    dataplane_id: 'dp',
    metadata: { constraints: {}, created_at: '2026-10-22T10:00:00Z', source: 'System' },
    payload: { data: {} },
    status: { Published: { at: '2026-10-22T10:00:05Z' } },
    target: { id: 'dep', kind: 'Deployment' },
    version: 1,
    ...overrides,
  }
}

describe('outcomeOf', () => {
  it('reads work not yet handed over as running', () => {
    expect(outcomeOf('Pending')).toBe('running')
    expect(outcomeOf({ Leased: { until: '2026-10-22T10:00:00Z' } })).toBe('running')
    expect(outcomeOf({ Pulled: { agent_id: 'a', at: '2026-10-22T10:00:00Z' } })).toBe('running')
  })

  it('reads a published action as done', () => {
    expect(outcomeOf({ Published: { at: '2026-10-22T10:00:00Z' } })).toBe('done')
  })

  it('reads a failed action as failed', () => {
    expect(outcomeOf({ Failed: { at: '2026-10-22T10:00:00Z', reason: 'Timeout' } })).toBe('failed')
  })
})

describe('toActivity', () => {
  it('shows the versions of an upgrade', () => {
    const [entry] = toActivity([
      action({
        id: 'a',
        action_type: 'deployment.upgrade',
        payload: { data: { from_version: '0.9.0', to_version: '0.9.1' } },
      }),
    ])

    expect(entry).toMatchObject({ id: 'a', type: 'deployment.upgrade', detail: '0.9.0 → 0.9.1' })
  })

  it('has no detail when the payload names no versions', () => {
    expect(toActivity([action({ id: 'a' })])[0].detail).toBeUndefined()
    expect(
      toActivity([action({ id: 'b', action_type: 'deployment.upgrade', payload: { data: null } })])[0]
        .detail,
    ).toBeUndefined()
  })

  it('leaves the platform internals out', () => {
    expect(toActivity([action({ id: 'a', action_type: 'system.heartbeat' })])).toEqual([])
  })

  it('keeps the newest eight, newest first', () => {
    const actions = Array.from({ length: 10 }, (_, index) =>
      action({
        id: String(index),
        metadata: {
          constraints: {},
          created_at: new Date(Date.UTC(2026, 9, 1, index)).toISOString(),
          source: 'System',
        },
      }),
    )

    const entries = toActivity(actions)

    expect(entries).toHaveLength(8)
    expect(entries[0].id).toBe('9')
    expect(entries[7].id).toBe('2')
  })
})

describe('hostnameFromActions', () => {
  it('takes the hostname from the newest action that carries one', () => {
    const older = action({
      id: 'a',
      payload: { data: { hostname: 'old.example.com' } },
    })
    const newer = action({
      id: 'b',
      metadata: { constraints: {}, created_at: '2026-10-22T11:00:00Z', source: 'System' },
      payload: { data: { hostname: 'new.example.com' } },
    })

    expect(hostnameFromActions([older, newer])).toBe('new.example.com')
  })

  it('is undefined when no action carries one', () => {
    expect(hostnameFromActions([action({ id: 'a' })])).toBeUndefined()
  })
})
