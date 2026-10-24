import { describe, expect, it } from 'vitest'
import { legacyRealm } from './legacy-realm'

describe('the realm of an address from before it was in the path', () => {
  it('keeps the realm it is given', () => {
    expect(legacyRealm({ realm: 'acme' })).toEqual({ realm: 'acme' })
  })

  it('says nothing when there was none, so the default realm opens', () => {
    expect(legacyRealm({})).toEqual({})
  })

  it('drops anything that is not a name', () => {
    expect(legacyRealm({ realm: '' })).toEqual({})
    expect(legacyRealm({ realm: 42 })).toEqual({})
    expect(legacyRealm({ realm: ['a'] })).toEqual({})
  })
})
