import { describe, expect, it } from 'vitest'
import type { CreateDeploymentForm } from './create-deployment-request'
import { toCreateDeploymentRequest } from './create-deployment-request'

function form(overrides: Partial<CreateDeploymentForm> = {}): CreateDeploymentForm {
  return {
    name: 'acme api',
    kind: 'ferriskey',
    version: '0.5.0',
    environment: 'production',
    offer: 'standard',
    ...overrides,
  }
}

describe('what the form sends', () => {
  /**
   * The six infrastructure decisions the request used to carry are gone. The
   * platform derives the namespace and the sizing from the offer and decides
   * where the thing runs, and refuses a request that still names any of them
   * rather than half-honouring it.
   */
  it('carries nothing the platform decides for itself', () => {
    expect(toCreateDeploymentRequest(form())).toEqual({
      name: 'acme api',
      kind: 'ferriskey',
      version: '0.5.0',
      environment: 'production',
      offer: 'standard',
    })
  })

  it('carries the offer the customer chose, whichever it is', () => {
    expect(toCreateDeploymentRequest(form({ offer: 'private' })).offer).toBe('private')
  })

  /**
   * A region is the fleet seen from outside: choosing one is choosing which
   * cluster serves you, which is an operator's decision. The API refuses a
   * request that names one, so sending it would turn every create into a 400.
   */
  it('names no region, because where a deployment runs is not the customer’s call', () => {
    expect(toCreateDeploymentRequest(form())).not.toHaveProperty('region')
  })

  it('sends the customer cloud distribution with the exact profile', () => {
    const request = toCreateDeploymentRequest(
      form({
        distribution: {
          type: 'customer_cloud',
          credential_id: 'cred-1',
          region: 'fr-par',
          profile: {
            mode: 'standard',
            control_plane_id: 'mutualized',
            node_type: 'DEV1-M',
            min_nodes: 2,
            max_nodes: 4,
            replication: 2,
          },
        },
      }),
    )

    expect(JSON.parse(JSON.stringify(request.distribution))).toEqual({
      type: 'customer_cloud',
      credential_id: 'cred-1',
      region: 'fr-par',
      profile: {
        mode: 'standard',
        control_plane_id: 'mutualized',
        node_type: 'DEV1-M',
        min_nodes: 2,
        max_nodes: 4,
        replication: 2,
      },
    })
  })

  it('sends no distribution when the deployment stays on the shared platform', () => {
    expect(toCreateDeploymentRequest(form())).not.toHaveProperty('distribution')
  })
})
