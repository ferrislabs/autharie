import { openOffers, type OfferAvailability } from './offers'

export const FREE_OFFER = 'free' as const

export const FREE_OFFER_COPY = {
  engine: 'The free offer runs FerrisKey.',
  address:
    'Your realm is reached at its own address, built from the deployment name. The name cannot be changed afterwards.',
  price: 'Free',
}

export function isFreeOnly(catalogue: OfferAvailability[]): boolean {
  const open = openOffers(catalogue)
  return open.length === 1 && open[0].offer === FREE_OFFER
}
