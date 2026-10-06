import { describe, expect, it } from 'vitest'
import { normalise, priceOf, type Engine, type Hosting, type PlanId } from './model'
import { applyChange } from './selection'

const amount = (hosting: Hosting, engine: Engine, plan: PlanId, volume: number) =>
  priceOf({ hosting, engine, plan, volume })

describe('managed prices', () => {
  const cases: [PlanId, Engine, number, number][] = [
    ['starter', 'ferriskey', 1000, 0],
    ['starter', 'ferriskey', 5000, 49],
    ['starter', 'ferriskey', 10000, 99],
    ['starter', 'ferriskey', 25000, 179],
    ['starter', 'keycloak', 1000, 49],
    ['business', 'ferriskey', 10000, 225],
    ['business', 'ferriskey', 500000, 2690],
    ['business', 'keycloak', 500000, 3490],
    ['scale', 'ferriskey', 10000, 349],
    ['scale', 'ferriskey', 500000, 4169],
    ['scale', 'keycloak', 500000, 5409],
  ]

  it.each(cases)('%s on %s at %i accounts costs %i', (plan, engine, volume, expected) => {
    expect(amount('managed', engine, plan, volume)).toEqual({ kind: 'price', amount: expected })
  })

  it('prices Business below its first tier at the 10,000 tier', () => {
    expect(amount('managed', 'ferriskey', 'business', 1000)).toEqual({ kind: 'price', amount: 225 })
  })

  it('does not offer Starter above 25,000 accounts', () => {
    expect(amount('managed', 'ferriskey', 'starter', 50000)).toEqual({ kind: 'unavailable' })
  })
})

describe('in your cloud prices', () => {
  const cases: [PlanId, number, number][] = [
    ['starter', 70, 89],
    ['business', 269, 349],
    ['scale', 629, 799],
  ]

  it.each(cases)('%s is a flat fee whatever the volume', (plan, ferriskey, keycloak) => {
    for (const volume of [1000, 500000]) {
      expect(amount('byoc', 'ferriskey', plan, volume)).toEqual({ kind: 'price', amount: ferriskey })
      expect(amount('byoc', 'keycloak', plan, volume)).toEqual({ kind: 'price', amount: keycloak })
    }
  })
})

describe('normalise', () => {
  it('moves Starter above 25,000 accounts up to Business', () => {
    const moved = normalise({ hosting: 'managed', engine: 'ferriskey', plan: 'starter', volume: 50000 })
    expect(moved.plan).toBe('business')
  })

  it('leaves a choice that fits alone', () => {
    const fits = { hosting: 'managed', engine: 'ferriskey', plan: 'starter', volume: 25000 } as const
    expect(normalise(fits)).toBe(fits)
  })

  it('says so when a change moved the plan', () => {
    const start = { hosting: 'managed', engine: 'ferriskey', plan: 'starter', volume: 10000 } as const
    expect(applyChange(start, { volume: 100000 })).toEqual({
      selection: { ...start, plan: 'business', volume: 100000 },
      movedToBusiness: true,
    })
    expect(applyChange(start, { volume: 5000 }).movedToBusiness).toBe(false)
  })
})
