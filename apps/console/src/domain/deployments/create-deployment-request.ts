import type { Schemas } from '@/api/api.client'
import { isPaid, paidActionBlock, type BillingAccess } from '@/domain/billing/billing'
import { type Environment } from './types/deployment'
import type { Offer } from './offers'

export interface CreateDeploymentForm {
  name: string
  /**
   * Exact, taken from the catalogue. It used to be the string `latest`, which
   * the platform refuses: a deployment records the version it runs, and a tag
   * that moves would make that record a lie the next time it moved.
   */
  version: string
  environment: Environment
  /** What they are buying. The size and the isolation come with it. */
  offer: Offer
  tier?: string
}

export function toCreateDeploymentRequest(
  form: CreateDeploymentForm,
): Schemas.CreateDeploymentRequest {
  return {
    name: form.name,
    kind: 'ferriskey',
    version: form.version,
    environment: form.environment,
    offer: form.offer,
    ...(form.tier ? { tier: form.tier } : {}),
  }
}

export function createBlock(
  tier: Schemas.TierResponse | undefined,
  access: BillingAccess,
): string | null {
  return tier && isPaid(tier) ? paidActionBlock(access) : null
}
