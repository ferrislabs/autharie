import type { Schemas } from '@/api/api.client'

export type Offer = Schemas.Offer
export type OfferAvailability = Schemas.OfferAvailability

/**
 * What each offer is, in the words a customer uses.
 *
 * Only the copy lives here. What an offer gives, whether it shares a cluster
 * and which tier opens it all come from the platform -- restating any of that
 * in TypeScript would be a second table, and the one that drifts is always the
 * one nobody looks at.
 */
export const OFFER_COPY: Record<Offer, { label: string; description: string }> = {
  free: {
    label: 'Free',
    description: 'A FerrisKey of your own, small, with its database on a cluster shared with other free deployments.',
  },
  sandbox: {
    label: 'Sandbox',
    description: 'Enough to try things against. Shares a cluster with other organisations.',
  },
  standard: {
    label: 'Standard',
    description: 'The everyday one, for an identity provider people actually sign in to.',
  },
  scale: {
    label: 'Scale',
    description: 'Room to grow into, still on a shared cluster.',
  },
  private: {
    label: 'Private',
    description: 'A Kubernetes cluster of your own. Nothing else runs on it.',
  },
}

/** What an offer gives, in one line. */
export function describeResources(entry: OfferAvailability): string {
  const { cpu_millis, memory_mib, storage_gib } = entry.resources

  return `${formatCpu(cpu_millis)} · ${formatMemory(memory_mib)} · ${storage_gib} Gi disk`
}

function formatCpu(millis: number): string {
  return millis >= 1000 ? `${millis / 1000} vCPU` : `${millis}m vCPU`
}

function formatMemory(mib: number): string {
  return mib >= 1024 ? `${mib / 1024} Gi RAM` : `${mib} Mi RAM`
}

export function openOffers(catalogue: OfferAvailability[]): OfferAvailability[] {
  return catalogue.filter((entry) => entry.open)
}

export const HOW_TO_CHANGE_OFFER =
  'The offer follows the tier: change the tier on the subscription to move between Sandbox, Standard and Scale. Moving between the free tier and a paid one, or to or from BYOC, needs a new deployment.'
