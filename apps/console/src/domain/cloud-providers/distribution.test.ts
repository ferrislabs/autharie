import { describe, expect, it } from 'vitest'
import type { CloudCredential } from './types/cloud-provider'
import {
  customerCloudAvailability,
  describeDistribution,
  toDistributionRequest,
} from './distribution'

const credential: CloudCredential = {
  id: 'cred-1',
  label: 'prod',
  provider: 'scaleway',
  created_at: '2026-10-01T00:00:00Z',
  scope_checked_at: '2026-10-01T00:00:00Z',
}

describe('offering the customer cloud', () => {
  it('is hidden for a kind other than FerrisKey', () => {
    expect(customerCloudAvailability('keycloak', [credential])).toBe('hidden')
    expect(customerCloudAvailability('keycloak', [])).toBe('hidden')
  })

  it('points to the credentials page when there is none', () => {
    expect(customerCloudAvailability('ferriskey', [])).toBe('needs_credentials')
  })

  it('is available to FerrisKey with a credential', () => {
    expect(customerCloudAvailability('ferriskey', [credential])).toBe('available')
  })
})

describe('the distribution sent', () => {
  it('is the exact customer_cloud body', () => {
    const body = toDistributionRequest({
      credentialId: 'cred-1',
      region: 'nl-ams',
      profile: {
        mode: 'ha',
        controlPlaneId: 'dedicated-4',
        nodeType: 'PRO2-S',
        minNodes: 3,
        maxNodes: 5,
        replication: 2,
      },
    })

    expect(JSON.parse(JSON.stringify(body))).toEqual({
      type: 'customer_cloud',
      credential_id: 'cred-1',
      region: 'nl-ams',
      profile: {
        mode: 'ha',
        control_plane_id: 'dedicated-4',
        node_type: 'PRO2-S',
        min_nodes: 3,
        max_nodes: 5,
        replication: 2,
      },
    })
  })
})

describe('reading a distribution back', () => {
  const profile = {
    control_plane: { id: 'mutualized', kind: 'mutualized' as const, monthly_price: 0 },
    max_nodes: 4,
    min_nodes: 2,
    mode: 'standard' as const,
    node_type: 'DEV1-M',
    replication: 2,
  }

  it('describes each externally tagged shape', () => {
    expect(describeDistribution('shared')).toBe('Shared')
    expect(describeDistribution('self_hosted')).toBe('Self-hosted')
    expect(describeDistribution({ customer_cloud: { credential_id: 'c', profile } })).toBe(
      'Your cloud, standard, 2 to 4 nodes',
    )
  })
})
