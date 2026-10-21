import type { Schemas } from '@/api/api.client'
import { isPaid, paidActionBlock, type BillingAccess } from '@/domain/billing/billing'
import { type Environment } from './types/deployment'
import type { Offer } from './offers'
import type { Hosting } from './pricing/model'

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

export function apiHostingOf(hosting: Hosting): Schemas.Hosting {
  return hosting === 'byoc' ? 'customer_cloud' : 'shared'
}

export interface SubmissionInput {
  name: string
  version: string
  environment: Environment
  tier: Schemas.TierResponse | undefined
  gridOffer: Offer | undefined
  loading: boolean
  submitting: boolean
  block: string | null
}

function offerOf({ tier, gridOffer }: SubmissionInput): Offer | undefined {
  return tier ? tier.offer : gridOffer
}

export function canSubmit(input: SubmissionInput): boolean {
  return (
    offerOf(input) !== undefined &&
    input.name.trim() !== '' &&
    input.version !== '' &&
    !input.submitting &&
    !input.loading
  )
}

export function submission(input: SubmissionInput): CreateDeploymentForm | null {
  const offer = offerOf(input)
  if (offer === undefined || !canSubmit(input) || input.block !== null) return null

  return {
    name: input.name,
    version: input.version,
    environment: input.environment,
    offer,
    ...(input.tier ? { tier: input.tier.code } : {}),
  }
}
