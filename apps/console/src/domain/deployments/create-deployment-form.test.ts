import { describe, expect, it } from 'vitest'
import { toCreateDeploymentRequest } from './create-deployment-request'

describe('the request a new deployment sends', () => {
  it('carries no plan, volume or price', () => {
    const body = toCreateDeploymentRequest({
      name: 'auth',
      kind: 'ferriskey',
      version: '1.2.3',
      environment: 'production',
      offer: 'standard',
    })

    expect(body).toEqual({
      name: 'auth',
      kind: 'ferriskey',
      version: '1.2.3',
      environment: 'production',
      offer: 'standard',
    })
  })
})
