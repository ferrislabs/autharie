import type { Schemas } from '@/api/api.client'

export interface TenantRealmView {
  realm: string
  url?: string
  issuer?: string
  refusal?: string
}

export const TENANT_REALM_COPY = {
  title: 'Sign-in',
  url: 'Tenant URL',
  issuer: 'Issuer',
  noAddress: 'This installation publishes no address for realms yet.',
  refused: 'The shared instance refused this realm:',
}

export function tenantRealmView(
  realm: Schemas.TenantRealm | null | undefined
): TenantRealmView | undefined {
  if (!realm) return undefined

  return {
    realm: realm.name,
    ...(realm.url ? { url: realm.url } : {}),
    ...(realm.issuer ? { issuer: realm.issuer } : {}),
    ...(realm.refusal ? { refusal: realm.refusal } : {}),
  }
}
