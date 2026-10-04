import { describe, expect, it } from 'vitest'
import { formatEur } from './money'

describe('money', () => {
  it('formats minor units as euros with two decimals', () => {
    expect(formatEur(1250)).toBe('€12.50')
    expect(formatEur(5)).toBe('€0.05')
    expect(formatEur(0)).toBe('€0.00')
  })
})
