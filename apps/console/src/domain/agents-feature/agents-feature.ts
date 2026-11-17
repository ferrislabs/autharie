import type { Schemas } from '@/api/api.client'

export const AGENTS_FEATURE: Schemas.Feature = 'agents'

export function hasAgents(enabled: readonly string[] | null | undefined): boolean {
  return enabled?.includes(AGENTS_FEATURE) === true
}
