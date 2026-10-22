import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  SECRET_POLL_MS,
  SECRET_WAIT_LIMIT_MS,
  secretPanel,
  shouldPollSecret,
  type Rotation,
} from './secret'

const webhook = (over: Partial<Schemas.WebhookView> = {}) =>
  ({
    secret_version: null,
    state: { phase: 'Active', ferriskey_id: 'f' },
    requests: [],
    ...over,
  }) as unknown as Schemas.WebhookView

const T0 = 1_000_000
const rotation: Rotation = { requestId: 'r1', startedAt: T0, versionBefore: 1 }

describe('the signing secret panel', () => {
  it('polls every 3 seconds and gives up after 2 minutes', () => {
    expect(SECRET_POLL_MS).toBe(3000)
    expect(SECRET_WAIT_LIMIT_MS).toBe(120_000)
  })

  it('is generating while a new webhook has no secret', () => {
    const panel = secretPanel(webhook(), null, T0 + 1000, T0)
    expect(panel).toEqual({ kind: 'generating' })
    expect(shouldPollSecret(panel)).toBe(true)
  })

  it('stops waiting for a first secret after 2 minutes', () => {
    const panel = secretPanel(webhook(), null, T0 + SECRET_WAIT_LIMIT_MS, T0)
    expect(panel).toEqual({ kind: 'stalled', rotating: false })
    expect(shouldPollSecret(panel)).toBe(false)
  })

  it('has nothing to wait for on a failed webhook', () => {
    const panel = secretPanel(webhook({ state: { phase: 'Failed', reason: 'x' } }), null, T0, T0)
    expect(panel).toEqual({ kind: 'unavailable' })
    expect(shouldPollSecret(panel)).toBe(false)
  })

  it('is ready with a version once there is a secret', () => {
    const panel = secretPanel(webhook({ secret_version: 2 }), null, T0, T0)
    expect(panel).toEqual({ kind: 'ready', version: 2 })
    expect(shouldPollSecret(panel)).toBe(false)
  })

  it('waits for the version to increase after a rotation', () => {
    const panel = secretPanel(webhook({ secret_version: 1 }), rotation, T0 + 5000, T0)
    expect(panel).toEqual({ kind: 'rotating' })
    expect(shouldPollSecret(panel)).toBe(true)
  })

  it('is ready again as soon as the version changes', () => {
    const panel = secretPanel(webhook({ secret_version: 2 }), rotation, T0 + 5000, T0)
    expect(panel).toEqual({ kind: 'ready', version: 2 })
  })

  it('stops after 2 minutes of rotating', () => {
    const panel = secretPanel(
      webhook({ secret_version: 1 }),
      rotation,
      T0 + SECRET_WAIT_LIMIT_MS + 1,
      T0
    )
    expect(panel).toEqual({ kind: 'stalled', rotating: true })
    expect(shouldPollSecret(panel)).toBe(false)
  })

  it('reports a rotation the data plane refused, with its message', () => {
    const failed = webhook({
      secret_version: 1,
      requests: [
        { id: 'r1', kind: 'rotate_secret', phase: 'failed', message: 'FerrisKey said no' },
      ] as Schemas.WebhookRequest[],
    })
    const panel = secretPanel(failed, rotation, T0 + 1000, T0)
    expect(panel).toEqual({ kind: 'rotation-failed', message: 'FerrisKey said no' })
    expect(shouldPollSecret(panel)).toBe(false)
  })

  it('ignores other requests', () => {
    const other = webhook({
      secret_version: 1,
      requests: [
        { id: 'zzz', kind: 'rotate_secret', phase: 'failed', message: 'old' },
      ] as Schemas.WebhookRequest[],
    })
    expect(secretPanel(other, rotation, T0 + 1000, T0)).toEqual({ kind: 'rotating' })
  })
})
