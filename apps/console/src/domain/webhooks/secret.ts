import type { Schemas } from '@/api/api.client'

export const SECRET_POLL_MS = 3000
export const SECRET_WAIT_LIMIT_MS = 120_000

export interface Rotation {
  requestId: string
  startedAt: number
  versionBefore: number | null
}

export type SecretPanel =
  | { kind: 'generating' }
  | { kind: 'rotating' }
  | { kind: 'ready'; version: number }
  | { kind: 'stalled'; rotating: boolean }
  | { kind: 'rotation-failed'; message: string }
  | { kind: 'unavailable' }

export function secretPanel(
  webhook: Schemas.WebhookView,
  rotation: Rotation | null,
  now: number,
  watchedSince: number
): SecretPanel {
  const version = webhook.secret_version ?? null

  if (rotation && version === rotation.versionBefore) {
    const request = webhook.requests.find((item) => item.id === rotation.requestId)
    if (request?.phase === 'failed') {
      return {
        kind: 'rotation-failed',
        message: request.message ?? 'The deployment could not generate a new secret.',
      }
    }
    return now - rotation.startedAt > SECRET_WAIT_LIMIT_MS
      ? { kind: 'stalled', rotating: true }
      : { kind: 'rotating' }
  }

  if (version !== null) return { kind: 'ready', version }
  if (webhook.state.phase === 'Failed') return { kind: 'unavailable' }

  return now - watchedSince >= SECRET_WAIT_LIMIT_MS
    ? { kind: 'stalled', rotating: false }
    : { kind: 'generating' }
}

export function shouldPollSecret(panel: SecretPanel): boolean {
  return panel.kind === 'generating' || panel.kind === 'rotating'
}
