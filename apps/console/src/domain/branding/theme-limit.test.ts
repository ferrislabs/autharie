import { describe, expect, it } from 'vitest'
import { MAX_THEMES, limitMessage, limitReached } from './theme-limit'

describe('the number of themes a realm holds', () => {
  it('is ten', () => {
    expect(MAX_THEMES).toBe(10)
  })

  it('is reached at ten and not before', () => {
    expect(limitReached(0)).toBe(false)
    expect(limitReached(9)).toBe(false)
    expect(limitReached(10)).toBe(true)
    expect(limitReached(11)).toBe(true)
  })

  it('says how many are used while there is room', () => {
    expect(limitMessage(3)).toBe('3 of 10 themes.')
    expect(limitMessage(9)).toBe('9 of 10 themes.')
  })

  it('says what to do once there is none left', () => {
    expect(limitMessage(10)).toBe(
      'This realm holds 10 themes, the most it can. Delete one to make room.'
    )
  })
})
