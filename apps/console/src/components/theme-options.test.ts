import { describe, expect, it } from 'vitest'
import { THEME_OPTIONS } from './theme-options'

describe('the themes a user can choose', () => {
  it('offers light, dark and following the system', () => {
    expect(THEME_OPTIONS.map((option) => option.value)).toEqual(['light', 'dark', 'system'])
  })

  it('names each one for a person rather than for the stylesheet', () => {
    expect(THEME_OPTIONS.map((option) => option.label)).toEqual(['Light', 'Dark', 'System'])
  })
})
