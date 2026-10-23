import { describe, expect, it } from 'vitest'
import { DEFAULT_REALM, realmFromSearch, validateRealmSearch } from './use-branding-realm'

describe('the realm in the address', () => {
  it('defaults to master', () => {
    expect(DEFAULT_REALM).toBe('master')
    expect(validateRealmSearch({})).toEqual({ realm: 'master' })
  })

  it('keeps the realm it is given', () => {
    expect(validateRealmSearch({ realm: 'acme' })).toEqual({ realm: 'acme' })
  })

  it('falls back to master on anything that is not a name', () => {
    expect(validateRealmSearch({ realm: '' })).toEqual({ realm: 'master' })
    expect(validateRealmSearch({ realm: 42 })).toEqual({ realm: 'master' })
    expect(validateRealmSearch({ realm: ['a'] })).toEqual({ realm: 'master' })
  })

  it('reads the realm from a search that may hold none', () => {
    expect(realmFromSearch({})).toBe('master')
    expect(realmFromSearch({ realm: 'acme' })).toBe('acme')
  })
})
