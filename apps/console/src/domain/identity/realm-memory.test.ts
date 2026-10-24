import { describe, expect, it } from 'vitest'
import { readRealm, rememberRealm } from './realm-memory'

function fakeStorage() {
  const held = new Map<string, string>()
  return {
    getItem: (key: string) => held.get(key) ?? null,
    setItem: (key: string, value: string) => void held.set(key, value),
  }
}

describe('the realm remembered for a deployment', () => {
  it('is master until a realm has been looked at', () => {
    expect(readRealm('dep-1', fakeStorage())).toBe('master')
  })

  it('is the last realm looked at', () => {
    const storage = fakeStorage()

    rememberRealm('dep-1', 'acme', storage)

    expect(readRealm('dep-1', storage)).toBe('acme')
  })

  it('is master where there is no storage to read', () => {
    expect(readRealm('dep-1', null)).toBe('master')
  })

  it('is kept apart per deployment', () => {
    const storage = fakeStorage()

    rememberRealm('dep-1', 'acme', storage)

    expect(readRealm('dep-2', storage)).toBe('master')
  })
})
