import type { Schemas } from '@/api/api.client'

export type AdministratorCredentialState = 'pending' | 'available' | 'revealed' | 'expired'

export interface TenantAdministratorView {
  username?: string
  credential: AdministratorCredentialState
  expiresAt?: string
}

export interface TenantRealmView {
  realm: string
  url?: string
  issuer?: string
  console?: string
  refusal?: string
  administrator?: TenantAdministratorView
}

export interface RevealedAdministrator {
  username: string
  password: string
}

export const TENANT_REALM_COPY = {
  title: 'Sign-in',
  url: 'Tenant URL',
  issuer: 'Issuer',
  administer: 'Administer',
  administerHint: 'Opens the administration console of your realm.',
  noAddress: 'This installation publishes no address for realms yet.',
  refused: 'The shared instance refused this realm:',
}

export const ADMINISTRATOR_COPY = {
  title: 'Administrator',
  username: 'Username',
  password: 'Temporary password',
  reveal: 'Reveal the administrator credential',
  revealing: 'Revealing…',
  copyUsername: 'Copy username',
  copyPassword: 'Copy password',
  shownOnce:
    'The password is shown once and is not kept: copy it now. Your realm asks for a new one at the first sign-in.',
  beforeReveal:
    'The first administrator of your realm is ready. Its password can be revealed once, by an administrator of this organisation.',
  pending: 'The first administrator of your realm is being created.',
  revealed:
    'The administrator credential was already revealed. Sign in with the password chosen at the first sign-in.',
  expired:
    'The administrator credential expired before anyone revealed it and is no longer kept. Contact support to have it reset.',
}

function administratorView(
  administrator: Schemas.TenantAdministrator | null | undefined
): TenantAdministratorView {
  if (!administrator) return { credential: 'pending' }

  return {
    username: administrator.username,
    credential: administrator.credential,
    expiresAt: administrator.expires_at,
  }
}

export function tenantRealmView(
  realm: Schemas.TenantRealm | null | undefined
): TenantRealmView | undefined {
  if (!realm) return undefined

  const administrator = realm.refusal ? undefined : administratorView(realm.administrator)

  return {
    realm: realm.name,
    ...(realm.url ? { url: realm.url } : {}),
    ...(realm.issuer ? { issuer: realm.issuer } : {}),
    ...(realm.console ? { console: realm.console } : {}),
    ...(realm.refusal ? { refusal: realm.refusal } : {}),
    ...(administrator ? { administrator } : {}),
  }
}
