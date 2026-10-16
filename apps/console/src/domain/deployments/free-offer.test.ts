import { describe, expect, it } from 'vitest'
import type { OfferAvailability } from './offers'
import { isFreeOnly, lockedOffers } from './free-offer'

function entry(overrides: Partial<OfferAvailability>): OfferAvailability {
  return {
    offer: 'standard',
    open: true,
    resources: { cpu_millis: 500, memory_mib: 1024, storage_gib: 1 },
    shares_a_cluster: true,
    ...overrides,
  } as OfferAvailability
}

const freeTier: OfferAvailability[] = [
  entry({ offer: 'free', open: true }),
  entry({ offer: 'sandbox', open: false, opened_by: 'Starter' }),
  entry({ offer: 'private', open: false, opened_by: 'Enterprise', shares_a_cluster: false }),
]

describe('the free tier', () => {
  it('is recognised when the free offer is the only one open', () => {
    expect(isFreeOnly(freeTier)).toBe(true)
  })

  it('is not a paying tier, even one that also sees a closed offer', () => {
    expect(
      isFreeOnly([
        entry({ offer: 'sandbox', open: true }),
        entry({ offer: 'standard', open: true }),
        entry({ offer: 'private', open: false, opened_by: 'Enterprise' }),
      ])
    ).toBe(false)
  })

  it('is not assumed while the catalogue is empty', () => {
    expect(isFreeOnly([])).toBe(false)
  })

  it('sees the sandbox locked behind the tier that opens it', () => {
    const locked = lockedOffers(freeTier)

    expect(locked.map((offer) => offer.label)).toEqual(['Sandbox', 'Private'])
    expect(locked[0].reason).toBe('Available from the Starter plan.')
    expect(locked[1].reason).toBe('Available from the Enterprise plan.')
  })
})
