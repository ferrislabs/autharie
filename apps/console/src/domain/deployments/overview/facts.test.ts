import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { adminUrlOf, endpointOf, formatResources, issuerOf, latestVersionOf, offerLabel } from './facts'

describe('formatResources', () => {
  it('writes a small free instance the way the offer reads', () => {
    expect(formatResources({ cpu_millis: 100, memory_mib: 256, storage_gib: 1 })).toBe(
      '0.1 vCPU · 256 MiB · 1 GiB database',
    )
  })

  it('counts whole gibibytes of memory in GiB and whole cores without a decimal', () => {
    expect(formatResources({ cpu_millis: 2000, memory_mib: 4096, storage_gib: 20 })).toBe(
      '2 vCPU · 4 GiB · 20 GiB database',
    )
  })

  it('keeps a quarter core readable', () => {
    expect(formatResources({ cpu_millis: 250, memory_mib: 1536, storage_gib: 5 })).toBe(
      '0.25 vCPU · 1536 MiB · 5 GiB database',
    )
  })
})

describe('endpoint', () => {
  it('is https on the hostname the platform applied', () => {
    expect(endpointOf('acme.autharie.fr')).toBe('https://acme.autharie.fr')
  })

  it('is absent without a hostname', () => {
    expect(endpointOf(undefined)).toBeUndefined()
  })

  it('puts a FerrisKey issuer under /api and a Keycloak one at the root', () => {
    expect(issuerOf('https://a.example', 'ferriskey')).toBe('https://a.example/api/realms/master')
    expect(issuerOf('https://a.example', 'keycloak')).toBe('https://a.example/realms/master')
    expect(issuerOf(undefined, 'ferriskey')).toBeUndefined()
  })

  it('offers the admin console for FerrisKey only', () => {
    expect(adminUrlOf('https://a.example', 'ferriskey')).toBe('https://a.example/admin')
    expect(adminUrlOf('https://a.example', 'keycloak')).toBeUndefined()
    expect(adminUrlOf(undefined, 'ferriskey')).toBeUndefined()
  })
})

describe('offerLabel', () => {
  it('names the offer a customer chose', () => {
    expect(offerLabel('standard')).toBe('Standard')
  })

  it('calls an instance with no offer custom', () => {
    expect(offerLabel(null)).toBe('Custom')
    expect(offerLabel(undefined)).toBe('Custom')
  })
})

describe('latestVersionOf', () => {
  function release(version: string, eligible = true): Schemas.ReleaseAvailability {
    return {
      eligible,
      id: { kind: 'ferriskey', version },
      status: 'available',
    } as unknown as Schemas.ReleaseAvailability
  }

  it('is the version on offer ahead of the current one', () => {
    expect(latestVersionOf('0.9.0', [release('0.9.0'), release('0.9.1')])).toBe('0.9.1')
  })

  it('is undefined when nothing newer is on offer', () => {
    expect(latestVersionOf('0.9.1', [release('0.9.1')])).toBeUndefined()
    expect(latestVersionOf('0.9.0', [release('0.9.1', false)])).toBeUndefined()
  })
})
