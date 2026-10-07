import { describe, expect, it } from 'vitest'
import type { Offer, OfferAvailability } from '../offers'
import { offerFor, resolveOffer } from './offer-for'
import type { PlanId } from './model'

describe('the offer a plan and a volume start on', () => {
  const table: [PlanId, number | null, Offer][] = [
    ['starter', 1000, 'sandbox'],
    ['business', 1000, 'sandbox'],
    ['starter', 5000, 'standard'],
    ['business', 25000, 'standard'],
    ['business', 50000, 'scale'],
    ['business', 500000, 'scale'],
    ['scale', 1000, 'scale'],
    ['scale', 10000, 'scale'],
    ['starter', null, 'standard'],
    ['business', null, 'standard'],
    ['scale', null, 'scale'],
  ]

  it.each(table)('%s at %s gives %s', (plan, volume, expected) => {
    expect(offerFor(plan, volume)).toBe(expected)
  })

  it('never derives the private offer', () => {
    for (const plan of ['starter', 'business', 'scale'] as PlanId[]) {
      for (const volume of [1000, 5000, 25000, 500000, null]) {
        expect(offerFor(plan, volume)).not.toBe('private')
      }
    }
  })
})

function entry(offer: Offer, open: boolean, opened_by?: string): OfferAvailability {
  return {
    offer,
    open,
    opened_by,
    resources: { cpu_millis: 500, memory_mib: 1024, storage_gib: 1 },
    shares_a_cluster: true,
  } as OfferAvailability
}

const catalogue = [
  entry('sandbox', true),
  entry('standard', false, 'SCALE'),
  entry('scale', false, 'SCALE'),
  entry('private', false),
]

describe('resolving the offer against what the platform opens', () => {
  it('uses the derived offer when it is open', () => {
    expect(resolveOffer('sandbox', catalogue)).toEqual({ offer: 'sandbox' })
  })

  it('selects the first open offer and says why when the derived one is closed', () => {
    const resolved = resolveOffer('standard', catalogue)
    expect(resolved.offer).toBe('sandbox')
    expect(resolved.because).toBe(
      'Standard fits this choice, but it is closed. Available from the Scale plan. Sandbox is selected instead.',
    )
  })

  it('selects nothing when nothing is open', () => {
    expect(resolveOffer('standard', [entry('sandbox', false)]).offer).toBeUndefined()
    expect(resolveOffer('standard', []).offer).toBeUndefined()
  })
})
