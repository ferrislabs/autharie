import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { ApiRequestError } from '@/api/api.fetch'
import { applyMode, initialDraft } from './profile'
import {
  RESIZE_FAILED,
  canApply,
  currentCredentialId,
  currentProfile,
  describeProfile,
  draftFromProfile,
  isResizable,
  isUnchanged,
  replicasBelowInUse,
  resizeFailure,
} from './resize'
import type { ProviderOffers } from './types/cloud-provider'

const offers: ProviderOffers = {
  control_planes: [
    { id: 'mutualized', kind: 'mutualized', monthly_price: 0 },
    { id: 'dedicated-4', kind: 'dedicated', monthly_price: 3500 },
  ],
  node_types: [
    { node_type: 'PRO2-S', monthly_price: 7200 },
    { node_type: 'PRO2-M', monthly_price: 14400 },
  ],
}

const dev: Schemas.ClusterProfile = {
  mode: 'dev',
  control_plane: { id: 'mutualized', kind: 'mutualized', monthly_price: 0 },
  node_type: 'PRO2-S',
  min_nodes: 1,
  max_nodes: 1,
  replication: 1,
}

const ha: Schemas.ClusterProfile = { ...dev, mode: 'ha', min_nodes: 3, max_nodes: 5, replication: 2 }

const customerCloud = (profile: Schemas.ClusterProfile): Schemas.Distribution => ({
  customer_cloud: { credential_id: 'cred-1', profile },
})

describe('which deployments can be resized', () => {
  it('needs a customer cloud deployment whose cluster is ready', () => {
    expect(isResizable({ distribution: customerCloud(dev) }, { status: 'ready' })).toBe(true)
    expect(isResizable({ distribution: customerCloud(dev) }, { status: 'provisioning' })).toBe(false)
    expect(isResizable({ distribution: customerCloud(dev) }, { status: 'failed' })).toBe(false)
    expect(isResizable({ distribution: customerCloud(dev) }, undefined)).toBe(false)
    expect(isResizable({ distribution: 'shared' }, { status: 'ready' })).toBe(false)
  })

  it('reads the profile and the credential from the distribution', () => {
    expect(currentProfile(customerCloud(dev))).toEqual(dev)
    expect(currentCredentialId(customerCloud(dev))).toBe('cred-1')
    expect(currentProfile('shared')).toBeUndefined()
    expect(currentCredentialId('self_hosted')).toBeUndefined()
  })
})

describe('the draft of a resize', () => {
  it('starts from the profile in force', () => {
    expect(draftFromProfile(ha)).toEqual({
      mode: 'ha',
      controlPlaneId: 'mutualized',
      nodeType: 'PRO2-S',
      minNodes: 3,
      maxNodes: 5,
      replication: 2,
    })
    expect(isUnchanged(draftFromProfile(ha), ha)).toBe(true)
  })

  it('cannot be applied until something changed', () => {
    expect(canApply(draftFromProfile(dev), dev, offers)).toBe(false)
  })

  it('can move dev to standard with the pre-filled range', () => {
    const draft = applyMode(draftFromProfile(dev), 'standard', offers)

    expect(canApply(draft, dev, offers)).toBe(true)
    expect(draft).toMatchObject({ mode: 'standard', minNodes: 2, replication: 2, nodeType: 'PRO2-S' })
  })

  it('refuses to take replicas away while they run', () => {
    const draft = applyMode(draftFromProfile(ha), 'dev', offers)

    expect(replicasBelowInUse(draft, ha)).toBe('2 replicas are running and Dev would allow 1.')
    expect(canApply(draft, ha, offers)).toBe(false)
  })

  it('never changes the node type', () => {
    const draft = { ...applyMode(draftFromProfile(dev), 'standard', offers), nodeType: 'PRO2-M' }

    expect(canApply(draft, dev, offers)).toBe(false)
  })

  it('keeps an incomplete draft from being applied', () => {
    const draft = { ...applyMode(draftFromProfile(dev), 'standard', offers), minNodes: 1 }

    expect(canApply(draft, dev, offers)).toBe(false)
  })

  it('starts from a profile whose control plane is not in the catalogue as incomplete', () => {
    const draft = { ...applyMode(initialDraft(offers), 'standard', offers), controlPlaneId: 'gone' }

    expect(canApply(draft, dev, offers)).toBe(false)
  })
})

describe('describing a profile', () => {
  it('names the mode, the nodes, the node type and the replicas', () => {
    expect(describeProfile(dev)).toBe('Dev, 1 node of PRO2-S, 1 replica')
    expect(describeProfile(ha)).toBe('High availability, 3 to 5 nodes of PRO2-S, 2 replicas')
  })
})

describe('the failure of a resize', () => {
  it('shows the refusal of the profile and of the state of the cluster as the server wrote them', () => {
    expect(resizeFailure(new ApiRequestError(422, '2 replicas are running and dev allows 1'))).toBe(
      '2 replicas are running and dev allows 1',
    )
    expect(resizeFailure(new ApiRequestError(409, 'the cluster of deployment x is provisioning'))).toBe(
      'the cluster of deployment x is provisioning',
    )
  })

  it('says it plainly when anything else went wrong', () => {
    expect(resizeFailure(new ApiRequestError(500, 'boom'))).toBe(RESIZE_FAILED)
    expect(resizeFailure(new Error('network'))).toBe(RESIZE_FAILED)
  })

  it('says nothing before anything failed', () => {
    expect(resizeFailure(null)).toBeUndefined()
  })
})
