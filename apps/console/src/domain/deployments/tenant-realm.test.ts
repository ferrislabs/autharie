import { describe, expect, it } from 'vitest'
import { tenantRealmView } from './tenant-realm'

describe('the tenant realm of a deployment', () => {
  it('is absent for a deployment with an instance of its own', () => {
    expect(tenantRealmView(undefined)).toBeUndefined()
    expect(tenantRealmView(null)).toBeUndefined()
  })

  it('carries the tenant URL, the issuer and the administration console the platform built', () => {
    expect(
      tenantRealmView({
        name: 'acme',
        url: 'https://acme.autharie.fr',
        issuer: 'https://acme.autharie.fr/api/realms/acme',
        console: 'https://acme.autharie.fr/realms/acme/overview',
        refusal: null,
      })
    ).toEqual({
      realm: 'acme',
      url: 'https://acme.autharie.fr',
      issuer: 'https://acme.autharie.fr/api/realms/acme',
      console: 'https://acme.autharie.fr/realms/acme/overview',
      administrator: { credential: 'none', stale: false },
    })
  })

  it('says why the realm was refused, and offers no administrator for it', () => {
    const view = tenantRealmView({ name: 'acme', refusal: 'the realm name is taken' })

    expect(view?.refusal).toBe('the realm name is taken')
    expect(view?.administrator).toBeUndefined()
  })

  it('has no address when the installation publishes none', () => {
    const view = tenantRealmView({ name: 'acme', url: null, issuer: null, console: null })

    expect(view?.url).toBeUndefined()
    expect(view?.issuer).toBeUndefined()
    expect(view?.console).toBeUndefined()
  })

  it('tells whether the administrator credential can still be revealed', () => {
    const view = tenantRealmView({
      name: 'acme',
      administrator: {
        username: 'admin',
        credential: 'revealed',
        stale: false,
        requested_at: '2026-10-13T12:00:00Z',
        expires_at: '2026-10-20T12:00:00Z',
      },
    })

    expect(view?.administrator).toEqual({
      username: 'admin',
      credential: 'revealed',
      stale: false,
      expiresAt: '2026-10-20T12:00:00Z',
    })
  })
})
