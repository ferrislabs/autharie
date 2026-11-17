import { describe, expect, it } from 'vitest'

import { isUnsubstituted, usableLink } from './config-app'

describe('isUnsubstituted', () => {
  it('recognises the placeholders config.json ships with', () => {
    expect(isUnsubstituted('${OIDC_ISSUER_URL}')).toBe(true)
    expect(isUnsubstituted('${OIDC_CLIENT_ID}')).toBe(true)
    expect(isUnsubstituted('${API_URL}')).toBe(true)
  })

  it('accepts a substituted value', () => {
    expect(isUnsubstituted('http://localhost:3334/realms/autharie')).toBe(false)
    expect(isUnsubstituted('console')).toBe(false)
  })

  /// A realm or client id is free to contain a dollar or a brace without the
  /// whole value being a placeholder.
  it('does not reject a value that merely contains the syntax', () => {
    expect(isUnsubstituted('http://localhost/realms/${weird}')).toBe(false)
    expect(isUnsubstituted('client-${x}-id')).toBe(false)
  })

  it('ignores anything that is not a string', () => {
    expect(isUnsubstituted(undefined)).toBe(false)
    expect(isUnsubstituted(null)).toBe(false)
    expect(isUnsubstituted(42)).toBe(false)
  })
})

describe('usableLink', () => {
  it('keeps a configured address', () => {
    expect(usableLink('https://docs.example.com/autharie')).toBe('https://docs.example.com/autharie')
    expect(usableLink('  mailto:help@example.com ')).toBe('mailto:help@example.com')
  })

  it('hides the link when nothing was configured', () => {
    expect(usableLink(undefined)).toBeUndefined()
    expect(usableLink(null)).toBeUndefined()
    expect(usableLink('')).toBeUndefined()
    expect(usableLink('   ')).toBeUndefined()
  })

  it('hides the link when the placeholder was never substituted', () => {
    expect(usableLink('${SUPPORT_URL}')).toBeUndefined()
    expect(usableLink('${DOCS_URL}')).toBeUndefined()
  })

  it('refuses an address that is not a web or mail link', () => {
    expect(usableLink('javascript:alert(1)')).toBeUndefined()
  })
})
