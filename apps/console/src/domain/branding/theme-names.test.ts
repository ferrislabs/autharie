import { describe, expect, it } from 'vitest'
import { availableName, copyName } from './theme-names'

describe('naming a copy', () => {
  it('adds (copy)', () => {
    expect(copyName('Autumn', ['Autumn'])).toBe('Autumn (copy)')
  })

  it('counts when the copy is taken too', () => {
    expect(copyName('Autumn', ['Autumn', 'Autumn (copy)'])).toBe('Autumn (copy 2)')
    expect(copyName('Autumn', ['Autumn', 'Autumn (copy)', 'Autumn (copy 2)'])).toBe(
      'Autumn (copy 3)'
    )
  })

  it('compares without regard to case or spaces, as the platform does', () => {
    expect(copyName('Autumn', ['autumn (COPY) '])).toBe('Autumn (copy 2)')
  })

  it('stays within 80 characters', () => {
    const long = 'a'.repeat(80)
    const name = copyName(long, [long])

    expect(Array.from(name)).toHaveLength(80)
    expect(name.endsWith(' (copy)')).toBe(true)
  })
})

describe('keeping a name when it is free', () => {
  it('returns the name as it is', () => {
    expect(availableName(' Autumn ', ['Winter'])).toBe('Autumn')
  })

  it('falls back to a copy name when it is taken', () => {
    expect(availableName('Autumn', ['AUTUMN'])).toBe('Autumn (copy)')
  })
})
