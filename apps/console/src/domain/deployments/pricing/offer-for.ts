import type { Offer, OfferAvailability } from '../offers'
import type { PlanId } from './model'
import { STARTER_MAX_VOLUME, VOLUMES } from './model'

/*
 * Which platform offer a plan and an expected volume start on. This is a product
 * assumption, not something the platform decides, and the owner will adjust it:
 *
 *   plan scale                    -> scale
 *   up to 1,000 accounts          -> sandbox
 *   up to 25,000 accounts         -> standard
 *   above 25,000 accounts         -> scale
 *   unlimited accounts (in your cloud), any other plan -> standard
 *
 * The private, dedicated offer is never derived, so it is not offered here.
 */
export function offerFor(plan: PlanId, volume: number | null): Offer {
  if (plan === 'scale') return 'scale'
  if (volume === null) return 'standard'
  if (volume <= 1000) return 'sandbox'
  if (volume <= 25000) return 'standard'
  return 'scale'
}

export function isOpen(offer: Offer, catalogue: OfferAvailability[]): boolean {
  return catalogue.some((entry) => entry.offer === offer && entry.open)
}

export function offersOfPlan(plan: PlanId): Offer[] {
  const volumes = plan === 'starter' ? VOLUMES.filter((v) => v <= STARTER_MAX_VOLUME) : VOLUMES
  return [...new Set(volumes.map((volume) => offerFor(plan, volume)))]
}

export function planIsOpen(plan: PlanId, catalogue: OfferAvailability[]): boolean {
  return offersOfPlan(plan).some((offer) => isOpen(offer, catalogue))
}

const PLAN_PREFERENCE: PlanId[] = ['business', 'starter', 'scale']

export function firstOpenPlan(catalogue: OfferAvailability[]): PlanId | undefined {
  return PLAN_PREFERENCE.find((plan) => planIsOpen(plan, catalogue))
}
