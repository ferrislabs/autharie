import type { Schemas } from '@/api/api.client'

export type AdministratorCredentialState = 'none' | 'issuing' | 'ready' | 'revealed' | 'expired'

export interface TenantAdministratorView {
  credential: AdministratorCredentialState
  stale: boolean
  username?: string
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
  generate: 'Generate the administrator credential',
  regenerate: 'Regenerate',
  confirmRegenerate: 'Replace the password',
  cancel: 'Cancel',
  regenerateWarning:
    'A new temporary password replaces the current one: whoever signs in with the current administrator password will no longer be able to.',
  reveal: 'Reveal',
  revealing: 'Revealing…',
  copyUsername: 'Copy username',
  copyPassword: 'Copy password',
  none: 'Your realm has an administrator account named admin. Generate its credential to sign in to the administration console.',
  issuing: 'The credential is being generated on your realm.',
  stale: 'The credential has not arrived. Generate it again.',
  ready: 'A credential is ready. It can be revealed once.',
  shownOnce:
    'The password is shown once and is not kept: copy it now. Your realm asks for a new one at the first sign-in.',
  revealed:
    'The credential was revealed. If it was lost, regenerate it: the current password stops working.',
  expired: 'The credential expired before anyone revealed it. Regenerate it.',
  unreadable:
    'The credential was revealed but could not be read, and it is not kept. Regenerate it.',
}

export function waitingForAdministrator(
  administrator: Pick<Schemas.TenantAdministrator, 'credential' | 'stale'> | null | undefined
): boolean {
  return administrator?.credential === 'issuing' && !administrator.stale
}

function administratorView(
  administrator: Schemas.TenantAdministrator | null | undefined
): TenantAdministratorView {
  if (!administrator) return { credential: 'none', stale: false }

  return {
    credential: administrator.credential,
    stale: administrator.stale,
    ...(administrator.username ? { username: administrator.username } : {}),
    ...(administrator.expires_at ? { expiresAt: administrator.expires_at } : {}),
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
