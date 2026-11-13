import type { Schemas } from '@/api/api.client'

export type Quota = Schemas.Quota
export type QuotaKind = Schemas.QuotaKind

export const UNLIMITED: Quota = { type: 'unlimited' }

export function limited(max: number): Quota {
  return { type: 'limited', max }
}

export function describeQuota(quota: Quota): string {
  return quota.type === 'unlimited' ? 'Unlimited' : String(quota.max)
}

export type ParsedLimit = { ok: true; quota: Quota } | { ok: false; reason: string }

/**
 * What somebody typed into the override field.
 *
 * Whole numbers only, zero included: zero is a quota that allows none, and
 * refusing it here would hide the one way to stop a deployment from adding
 * anything. A decimal or a sign is refused rather than rounded, because a
 * limit that quietly became a different number is worse than an error.
 */
export function parseLimit(input: string): ParsedLimit {
  const text = input.trim()

  if (!/^\d+$/.test(text)) {
    return { ok: false, reason: 'Enter a whole number, 0 or more.' }
  }

  const max = Number(text)
  if (!Number.isSafeInteger(max) || max > 2_147_483_647) {
    return { ok: false, reason: 'That number is too large.' }
  }

  return { ok: true, quota: limited(max) }
}

/** The state of one row, for the badge beside it. */
export function sourceLabel(override: Quota | null | undefined): 'Override' | 'Default' {
  return override ? 'Override' : 'Default'
}

/** How many of a quota is used, as it reads on a deployment page. */
export function describeUsage(held: number, quota: Quota): string {
  return quota.type === 'unlimited' ? `${held}` : `${held} of ${quota.max}`
}

/** Whether the create button should be offered. */
export function hasRoom(held: number, quota: Quota | undefined): boolean {
  return !quota || quota.type === 'unlimited' || held < quota.max
}
