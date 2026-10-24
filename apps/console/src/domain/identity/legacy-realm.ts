/**
 * The realm of an address from before it was part of the path.
 *
 * Branding pages carried it as `?realm=`. Anything else in that place is not a
 * realm name, and is dropped rather than guessed at: the redirect then opens
 * the default realm.
 */
export function legacyRealm(search: Record<string, unknown>): { realm?: string } {
  return typeof search.realm === 'string' && search.realm !== '' ? { realm: search.realm } : {}
}
