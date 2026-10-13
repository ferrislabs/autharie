import { describe, expect, it } from 'vitest'
import { tenantRealmView } from './tenant-realm'

describe('the tenant realm of a deployment', () => {
  it('is absent for a deployment with an instance of its own', () => {
    expect(tenantRealmView(undefined)).toBeUndefined()
    expect(tenantRealmView(null)).toBeUndefined()
  })

  it('carries the tenant URL and the issuer the platform built', () => {
    expect(
      tenantRealmView({
        name: 'acme',
        url: 'https://acme.autharie.fr',
        issuer: 'https://acme.autharie.fr/api/realms/acme',
        refusal: null,
      })
    ).toEqual({
      realm: 'acme',
      url: 'https://acme.autharie.fr',
      issuer: 'https://acme.autharie.fr/api/realms/acme',
    })
  })

  it('says why the realm was refused', () => {
    expect(tenantRealmView({ name: 'acme', refusal: 'the realm name is taken' })?.refusal).toBe(
      'the realm name is taken'
    )
  })

  it('has no address when the installation publishes none', () => {
    const view = tenantRealmView({ name: 'acme', url: null, issuer: null })

    expect(view?.url).toBeUndefined()
    expect(view?.issuer).toBeUndefined()
  })
})
