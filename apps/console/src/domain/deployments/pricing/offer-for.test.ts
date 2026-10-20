import { describe, expect, it } from 'vitest'
import type { Offer, OfferAvailability } from '../offers'
import { firstOpenPlan, isOpen, offerFor, offersOfPlan, planIsOpen } from './offer-for'
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

describe('what the platform opens', () => {
  it('lists the offers a plan can start on', () => {
    expect(offersOfPlan('starter')).toEqual(['sandbox', 'standard'])
    expect(offersOfPlan('business')).toEqual(['sandbox', 'standard', 'scale'])
    expect(offersOfPlan('scale')).toEqual(['scale'])
  })

  it('knows whether an offer is open', () => {
    expect(isOpen('sandbox', catalogue)).toBe(true)
    expect(isOpen('standard', catalogue)).toBe(false)
    expect(isOpen('private', [])).toBe(false)
  })

  it('does not let a plan be chosen when none of its offers is open', () => {
    expect(planIsOpen('starter', catalogue)).toBe(true)
    expect(planIsOpen('business', catalogue)).toBe(true)
    expect(planIsOpen('scale', catalogue)).toBe(false)
  })

  it('starts on the first plan that can be chosen', () => {
    expect(firstOpenPlan(catalogue)).toBe('business')
    expect(firstOpenPlan([entry('scale', true)])).toBe('business')
    expect(firstOpenPlan([entry('standard', false)])).toBeUndefined()
    expect(firstOpenPlan([])).toBeUndefined()
  })

  it('never substitutes another offer for a closed one', () => {
    expect(isOpen(offerFor('starter', 5000), catalogue)).toBe(false)
  })
})
