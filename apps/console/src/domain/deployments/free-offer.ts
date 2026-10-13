import { OFFER_COPY, whyClosed, type OfferAvailability } from './offers'

export const FREE_OFFER = 'free' as const

export const FREE_OFFER_COPY = {
  engine: 'The free offer runs FerrisKey.',
  address:
    'Your realm is reached at its own address, built from the deployment name. The name cannot be changed afterwards.',
  price: 'Free',
}

export interface LockedOffer {
  label: string
  description: string
  reason: string
}

export function isFreeOnly(catalogue: OfferAvailability[]): boolean {
  const open = catalogue.filter((entry) => entry.open)
  return open.length === 1 && open[0].offer === FREE_OFFER
}

export function lockedOffers(catalogue: OfferAvailability[]): LockedOffer[] {
  return catalogue
    .filter((entry) => !entry.open)
    .map((entry) => ({
      label: OFFER_COPY[entry.offer].label,
      description: OFFER_COPY[entry.offer].description,
      reason: whyClosed(entry) ?? '',
    }))
}
