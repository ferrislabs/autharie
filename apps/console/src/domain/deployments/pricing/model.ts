/*
 * Mirrors the website's pricing model, /opt/ferrislabs/autharie.fr/apps/website/src/pricing/model.ts.
 * It is a copy until the platform serves prices, and is to be replaced by what the platform serves.
 */

export const VOLUMES = [1000, 5000, 10000, 25000, 50000, 100000, 250000, 500000] as const

export type Hosting = 'managed' | 'byoc'
export type Engine = 'ferriskey' | 'keycloak'
export type PlanId = 'starter' | 'business' | 'scale'

export type Price = { kind: 'price'; amount: number } | { kind: 'unavailable' }

type Tiers = [number, number][]

const STARTER: Record<Engine, Tiers> = {
  ferriskey: [[1000, 0], [5000, 49], [10000, 99], [25000, 179]],
  keycloak: [[1000, 49], [5000, 69], [10000, 129], [25000, 239]],
}

const BUSINESS: Record<Engine, Tiers> = {
  ferriskey: [[10000, 225], [50000, 530], [100000, 890], [250000, 1699], [500000, 2690]],
  keycloak: [[10000, 289], [50000, 690], [100000, 1150], [250000, 2199], [500000, 3490]],
}

const SCALE: Record<Engine, Tiers> = {
  ferriskey: [[10000, 349], [50000, 829], [100000, 1379], [250000, 2629], [500000, 4169]],
  keycloak: [[10000, 449], [50000, 1069], [100000, 1779], [250000, 3409], [500000, 5409]],
}

export const STARTER_MAX_VOLUME = 25000

const BYOC: Record<PlanId, Record<Engine, number>> = {
  starter: { ferriskey: 70, keycloak: 89 },
  business: { ferriskey: 269, keycloak: 349 },
  scale: { ferriskey: 629, keycloak: 799 },
}

const TABLES: Record<PlanId, Record<Engine, Tiers>> = {
  starter: STARTER,
  business: BUSINESS,
  scale: SCALE,
}

export interface Selection {
  hosting: Hosting
  engine: Engine
  plan: PlanId
  volume: number
}

export function priceOf({ hosting, engine, plan, volume }: Selection): Price {
  if (hosting === 'byoc') return { kind: 'price', amount: BYOC[plan][engine] }

  const found = TABLES[plan][engine].find(([upTo]) => volume <= upTo)
  return found ? { kind: 'price', amount: found[1] } : { kind: 'unavailable' }
}

export function normalise(selection: Selection): Selection {
  return priceOf(selection).kind === 'unavailable' ? { ...selection, plan: 'business' } : selection
}
