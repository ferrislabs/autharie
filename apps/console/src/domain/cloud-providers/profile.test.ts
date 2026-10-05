import { describe, expect, it } from 'vitest'
import type { ProviderOffers } from './types/cloud-provider'
import {
  applyMode,
  controlPlaneAllowed,
  initialDraft,
  isComplete,
  locksNodes,
  setMaxNodes,
  setMinNodes,
  toProfileRequest,
} from './profile'

const offers: ProviderOffers = {
  control_planes: [
    { id: 'mutualized', kind: 'mutualized', monthly_price: 0 },
    { id: 'dedicated-4', kind: 'dedicated', monthly_price: 3500 },
  ],
  node_types: [{ node_type: 'DEV1-M', monthly_price: 1500 }],
}

describe('picking a mode', () => {
  it('pre-fills dev with one node, one replica and the mutualized control plane', () => {
    const draft = applyMode(initialDraft(offers), 'dev', offers)

    expect(draft).toMatchObject({
      mode: 'dev',
      minNodes: 1,
      maxNodes: 1,
      replication: 1,
      controlPlaneId: 'mutualized',
    })
  })

  it('pre-fills standard with two nodes at least and two replicas', () => {
    const draft = applyMode(initialDraft(offers), 'standard', offers)

    expect(draft).toMatchObject({ mode: 'standard', minNodes: 2, replication: 2 })
    expect(draft.maxNodes).toBeGreaterThanOrEqual(2)
  })

  it('pre-fills ha with three nodes at least and two replicas', () => {
    const draft = applyMode(initialDraft(offers), 'ha', offers)

    expect(draft).toMatchObject({ mode: 'ha', minNodes: 3, replication: 2 })
  })

  it('drops a dedicated control plane when going back to dev', () => {
    const standard = { ...applyMode(initialDraft(offers), 'standard', offers), controlPlaneId: 'dedicated-4' }

    expect(applyMode(standard, 'dev', offers).controlPlaneId).toBe('mutualized')
  })

  it('keeps the node type across modes', () => {
    const draft = { ...initialDraft(offers), nodeType: 'DEV1-M' }

    expect(applyMode(draft, 'ha', offers).nodeType).toBe('DEV1-M')
  })
})

describe('what a mode forbids', () => {
  it('makes a dedicated control plane impossible in dev', () => {
    expect(controlPlaneAllowed('dev', offers.control_planes[1])).toBe(false)
    expect(controlPlaneAllowed('dev', offers.control_planes[0])).toBe(true)
  })

  it('allows either control plane in standard and ha', () => {
    expect(controlPlaneAllowed('standard', offers.control_planes[1])).toBe(true)
    expect(controlPlaneAllowed('ha', offers.control_planes[1])).toBe(true)
  })

  it('fixes the node count in dev only', () => {
    expect(locksNodes('dev')).toBe(true)
    expect(locksNodes('standard')).toBe(false)
    expect(locksNodes('ha')).toBe(false)
  })

  it('refuses a dev draft that carries a dedicated control plane, whatever set it', () => {
    const draft = { ...applyMode(initialDraft(offers), 'dev', offers), nodeType: 'DEV1-M', controlPlaneId: 'dedicated-4' }

    expect(isComplete(draft, offers)).toBe(false)
  })

  it('keeps nodes at the mode floor and the maximum above the minimum', () => {
    const ha = applyMode(initialDraft(offers), 'ha', offers)

    expect(setMinNodes(ha, 1).minNodes).toBe(3)
    expect(setMaxNodes(setMinNodes(ha, 5), 4).maxNodes).toBe(5)
    expect(setMinNodes(setMaxNodes(ha, 4), 6).maxNodes).toBe(6)
  })

  it('does not move the nodes of a dev draft', () => {
    const dev = applyMode(initialDraft(offers), 'dev', offers)

    expect(setMinNodes(dev, 3)).toEqual(dev)
    expect(setMaxNodes(dev, 3)).toEqual(dev)
  })
})

describe('a complete profile', () => {
  const ready = { ...applyMode(initialDraft(offers), 'standard', offers), nodeType: 'DEV1-M' }

  it('needs a node type and a control plane from the catalogue', () => {
    expect(isComplete(ready, offers)).toBe(true)
    expect(isComplete({ ...ready, nodeType: '' }, offers)).toBe(false)
    expect(isComplete({ ...ready, nodeType: 'GONE' }, offers)).toBe(false)
    expect(isComplete({ ...ready, controlPlaneId: 'gone' }, offers)).toBe(false)
  })

  it('becomes the request body the API expects', () => {
    expect(toProfileRequest(ready)).toEqual({
      mode: 'standard',
      control_plane_id: 'mutualized',
      node_type: 'DEV1-M',
      min_nodes: 2,
      max_nodes: 4,
      replication: 2,
    })
  })
})
