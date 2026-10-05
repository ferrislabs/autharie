import type { Schemas } from '@/api/api.client'
import type { ClusterMode, ControlPlaneOffer, ProviderOffers } from './types/cloud-provider'

interface ModeRules {
  label: string
  description: string
  minNodes: number
  replication: number
  autoscaling: boolean
  dedicatedControlPlane: boolean
}

export const MODE_RULES: Record<ClusterMode, ModeRules> = {
  dev: {
    label: 'Dev',
    description: 'One node, one replica. To try the product at the lowest cost.',
    minNodes: 1,
    replication: 1,
    autoscaling: false,
    dedicatedControlPlane: false,
  },
  standard: {
    label: 'Standard',
    description: 'At least two nodes and two replicas, with autoscaling.',
    minNodes: 2,
    replication: 2,
    autoscaling: true,
    dedicatedControlPlane: true,
  },
  ha: {
    label: 'High availability',
    description: 'At least three nodes, two replicas and three database instances.',
    minNodes: 3,
    replication: 2,
    autoscaling: true,
    dedicatedControlPlane: true,
  },
}

export const MODES: ClusterMode[] = ['dev', 'standard', 'ha']

const AUTOSCALING_HEADROOM = 2

export interface ProfileDraft {
  mode: ClusterMode
  controlPlaneId: string
  nodeType: string
  minNodes: number
  maxNodes: number
  replication: number
}

export function locksNodes(mode: ClusterMode): boolean {
  return !MODE_RULES[mode].autoscaling
}

export function controlPlaneAllowed(mode: ClusterMode, offer: ControlPlaneOffer): boolean {
  return offer.kind === 'mutualized' || MODE_RULES[mode].dedicatedControlPlane
}

function firstAllowed(mode: ClusterMode, offers: ProviderOffers): string {
  return offers.control_planes.find((offer) => controlPlaneAllowed(mode, offer))?.id ?? ''
}

export function initialDraft(offers: ProviderOffers): ProfileDraft {
  return applyMode(
    {
      mode: 'dev',
      controlPlaneId: '',
      nodeType: offers.node_types[0]?.node_type ?? '',
      minNodes: 1,
      maxNodes: 1,
      replication: 1,
    },
    'dev',
    offers,
  )
}

export function applyMode(
  draft: ProfileDraft,
  mode: ClusterMode,
  offers: ProviderOffers,
): ProfileDraft {
  const rules = MODE_RULES[mode]
  const current = offers.control_planes.find((offer) => offer.id === draft.controlPlaneId)
  const keep = current !== undefined && controlPlaneAllowed(mode, current)

  return {
    ...draft,
    mode,
    minNodes: rules.minNodes,
    maxNodes: rules.autoscaling ? rules.minNodes + AUTOSCALING_HEADROOM : rules.minNodes,
    replication: rules.replication,
    controlPlaneId: keep ? draft.controlPlaneId : firstAllowed(mode, offers),
  }
}

export function setMinNodes(draft: ProfileDraft, value: number): ProfileDraft {
  if (locksNodes(draft.mode)) return draft
  const minNodes = Math.max(value, MODE_RULES[draft.mode].minNodes)

  return { ...draft, minNodes, maxNodes: Math.max(draft.maxNodes, minNodes) }
}

export function setMaxNodes(draft: ProfileDraft, value: number): ProfileDraft {
  if (locksNodes(draft.mode)) return draft

  return { ...draft, maxNodes: Math.max(value, draft.minNodes) }
}

export function isComplete(draft: ProfileDraft, offers: ProviderOffers): boolean {
  const controlPlane = offers.control_planes.find((offer) => offer.id === draft.controlPlaneId)
  const nodeType = offers.node_types.some((offer) => offer.node_type === draft.nodeType)
  const rules = MODE_RULES[draft.mode]

  return (
    controlPlane !== undefined &&
    controlPlaneAllowed(draft.mode, controlPlane) &&
    nodeType &&
    draft.minNodes >= rules.minNodes &&
    draft.maxNodes >= draft.minNodes &&
    (rules.autoscaling || draft.maxNodes === draft.minNodes) &&
    draft.replication <= draft.minNodes
  )
}

export function toProfileRequest(draft: ProfileDraft): Schemas.ClusterProfileRequest {
  return {
    mode: draft.mode,
    control_plane_id: draft.controlPlaneId,
    node_type: draft.nodeType,
    min_nodes: draft.minNodes,
    max_nodes: draft.maxNodes,
    replication: draft.replication,
  }
}
