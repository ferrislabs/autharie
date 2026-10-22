import type { Schemas } from '@/api/api.client'

export interface WebhookLimit {
  label: string
  atLimit: boolean
  reason: string | null
}

export function maxWebhooks(plan: Schemas.Plan): number {
  return plan === 'Free' ? 5 : 25
}

export function describeLimit(count: number, plan: Schemas.Plan | undefined): WebhookLimit {
  if (!plan) return { label: String(count), atLimit: false, reason: null }

  const max = maxWebhooks(plan)
  const atLimit = count >= max

  return {
    label: `${count} of ${max}`,
    atLimit,
    reason: atLimit
      ? `Your ${plan} plan holds ${max} webhooks per deployment. Delete one, or upgrade the plan, to add another.`
      : null,
  }
}
