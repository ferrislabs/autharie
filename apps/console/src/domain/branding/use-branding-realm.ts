import { useNavigate, useSearch } from '@tanstack/react-router'

export const DEFAULT_REALM = 'master'

export function realmFromSearch(search: Record<string, unknown>): string {
  return typeof search.realm === 'string' && search.realm !== '' ? search.realm : DEFAULT_REALM
}

export function validateRealmSearch(search: Record<string, unknown>): { realm: string } {
  return { realm: realmFromSearch(search) }
}

export function useBrandingRealm(): [string, (realm: string) => void] {
  const search = useSearch({ strict: false }) as Record<string, unknown>
  const navigate = useNavigate()

  return [
    realmFromSearch(search),
    (realm) => void navigate({ to: '.', search: (previous) => ({ ...previous, realm }), replace: true }),
  ]
}
