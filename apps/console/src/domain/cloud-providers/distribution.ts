import type { Schemas } from '@/api/api.client'
import type { DeploymentKind } from '@/domain/deployments/types/deployment'
import type { CloudCredential } from './types/cloud-provider'
import { toProfileRequest, type ProfileDraft } from './profile'

export type CustomerCloudAvailability = 'hidden' | 'needs_credentials' | 'available'

export function customerCloudAvailability(
  kind: DeploymentKind,
  credentials: CloudCredential[],
): CustomerCloudAvailability {
  if (kind !== 'ferriskey') return 'hidden'
  return credentials.length === 0 ? 'needs_credentials' : 'available'
}

export interface CustomerCloudChoice {
  credentialId: string
  region: string
  profile: ProfileDraft
}

export function toDistributionRequest(
  choice: CustomerCloudChoice,
): Schemas.DistributionRequest {
  return {
    type: 'customer_cloud',
    credential_id: choice.credentialId,
    region: choice.region,
    profile: toProfileRequest(choice.profile),
  }
}

export const REGIONS: Record<Schemas.Provider, string[]> = {
  scaleway: ['fr-par', 'nl-ams', 'pl-waw'],
}

export function describeDistribution(distribution: Schemas.Distribution): string {
  if (distribution === 'shared') return 'Shared'
  if (distribution === 'self_hosted') return 'Self-hosted'
  if ('pooled' in distribution) return 'Shared cell'
  const { mode, min_nodes, max_nodes } = distribution.customer_cloud.profile
  const nodes = min_nodes === max_nodes ? `${min_nodes}` : `${min_nodes} to ${max_nodes}`
  return `Your cloud, ${mode}, ${nodes} ${max_nodes === 1 ? 'node' : 'nodes'}`
}
