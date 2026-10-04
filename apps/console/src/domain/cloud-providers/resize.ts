import type { Schemas } from '@/api/api.client'
import { ApiRequestError } from '@/api/api.fetch'
import { isComplete, MODE_RULES, type ProfileDraft } from './profile'
import { isCustomerCloud } from './provisioning'
import type { ProviderOffers } from './types/cloud-provider'

export const RESIZE_FAILED = 'The cluster could not be resized. Try again in a moment.'

export function currentProfile(
  distribution: Schemas.Distribution,
): Schemas.ClusterProfile | undefined {
  return typeof distribution === 'object' && 'customer_cloud' in distribution
    ? distribution.customer_cloud.profile
    : undefined
}

export function currentCredentialId(distribution: Schemas.Distribution): string | undefined {
  return typeof distribution === 'object' && 'customer_cloud' in distribution
    ? distribution.customer_cloud.credential_id
    : undefined
}

export function isResizable(
  deployment: Pick<Schemas.Deployment, 'distribution'>,
  provisioning: Schemas.Provisioning | null | undefined,
): boolean {
  return isCustomerCloud(deployment.distribution) && provisioning?.status === 'ready'
}

export function draftFromProfile(profile: Schemas.ClusterProfile): ProfileDraft {
  return {
    mode: profile.mode,
    controlPlaneId: profile.control_plane.id,
    nodeType: profile.node_type,
    minNodes: profile.min_nodes,
    maxNodes: profile.max_nodes,
    replication: profile.replication,
  }
}

export function isUnchanged(draft: ProfileDraft, profile: Schemas.ClusterProfile): boolean {
  const current = draftFromProfile(profile)

  return (
    draft.mode === current.mode &&
    draft.controlPlaneId === current.controlPlaneId &&
    draft.minNodes === current.minNodes &&
    draft.maxNodes === current.maxNodes &&
    draft.replication === current.replication
  )
}

export function replicasBelowInUse(
  draft: ProfileDraft,
  profile: Schemas.ClusterProfile,
): string | undefined {
  if (draft.replication >= profile.replication) return undefined

  return `${profile.replication} replicas are running and ${MODE_RULES[draft.mode].label} would allow ${draft.replication}.`
}

export function canApply(
  draft: ProfileDraft,
  profile: Schemas.ClusterProfile,
  offers: ProviderOffers,
): boolean {
  return (
    draft.nodeType === profile.node_type &&
    !isUnchanged(draft, profile) &&
    replicasBelowInUse(draft, profile) === undefined &&
    isComplete(draft, offers)
  )
}

export function describeProfile(profile: Schemas.ClusterProfile): string {
  const nodes =
    profile.min_nodes === profile.max_nodes
      ? `${profile.min_nodes}`
      : `${profile.min_nodes} to ${profile.max_nodes}`

  return `${MODE_RULES[profile.mode].label}, ${nodes} ${profile.max_nodes === 1 ? 'node' : 'nodes'} of ${profile.node_type}, ${profile.replication} ${profile.replication === 1 ? 'replica' : 'replicas'}`
}

export function resizeFailure(error: unknown): string | undefined {
  if (error === null || error === undefined) return undefined
  if (error instanceof ApiRequestError && (error.status === 422 || error.status === 409)) {
    return error.message
  }

  return RESIZE_FAILED
}
