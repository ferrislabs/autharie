import { firstOpen, OFFER_COPY, whyClosed, type Offer, type OfferAvailability } from '../offers'
import type { PlanId } from './model'

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
 * The private, dedicated offer is never derived. It stays reachable from the
 * Offer section.
 */
export function offerFor(plan: PlanId, volume: number | null): Offer {
  if (plan === 'scale') return 'scale'
  if (volume === null) return 'standard'
  if (volume <= 1000) return 'sandbox'
  if (volume <= 25000) return 'standard'
  return 'scale'
}

export interface ResolvedOffer {
  offer: Offer | undefined
  because?: string
}

export function resolveOffer(
  derived: Offer,
  override: Offer | undefined,
  catalogue: OfferAvailability[],
): ResolvedOffer {
  const open = (offer: Offer) => catalogue.find((entry) => entry.offer === offer)?.open === true

  if (override && open(override)) return { offer: override }
  if (open(derived)) return { offer: derived }

  const fallback = firstOpen(catalogue)
  const closed = catalogue.find((entry) => entry.offer === derived)
  if (!fallback) return { offer: undefined }

  const reason = closed ? (whyClosed(closed) ?? '') : 'The platform does not offer it.'
  return {
    offer: fallback,
    because: `${OFFER_COPY[derived].label} fits this choice, but it is closed. ${reason} ${OFFER_COPY[fallback].label} is selected instead.`,
  }
}
