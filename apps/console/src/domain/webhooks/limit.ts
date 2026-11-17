import { describeUsage, hasRoom } from '@/domain/quotas/quota'
import type { Quota } from '@/domain/quotas/quota'

export interface WebhookLimit {
  label: string
  atLimit: boolean
  reason: string | null
}

/**
 * Where a deployment stands against its webhook quota.
 *
 * The quota is the one the server holds the deployment to: its offer's
 * default, or the override set for its organisation. It is not derived from
 * the plan here, because an override would then leave the button disabled
 * while the server accepts the create.
 */
export function describeLimit(count: number, quota: Quota | undefined): WebhookLimit {
  if (!quota) return { label: String(count), atLimit: false, reason: null }

  const atLimit = !hasRoom(count, quota)

  return {
    label: describeUsage(count, quota),
    atLimit,
    reason: atLimit
      ? quota.type === 'limited'
        ? `This deployment holds ${quota.max} webhooks. Delete one, or ask for a higher quota, to add another.`
        : null
      : null,
  }
}
