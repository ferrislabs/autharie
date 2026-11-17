import { describe, expect, it } from 'vitest'
import { heldLabel, isOverLimit } from './organisation-quota'

describe('the held count of an organisation quota', () => {
  it('reads as used of limit when limited', () => {
    expect(heldLabel({ held: 2, effective: { type: 'limited', max: 3 } })).toBe('2 of 3')
  })

  it('reads as the bare count when unlimited', () => {
    expect(heldLabel({ held: 7, effective: { type: 'unlimited' } })).toBe('7')
  })

  it('flags an organisation holding more than it may', () => {
    expect(isOverLimit({ held: 4, effective: { type: 'limited', max: 3 } })).toBe(true)
    expect(isOverLimit({ held: 3, effective: { type: 'limited', max: 3 } })).toBe(false)
    expect(isOverLimit({ held: 9, effective: { type: 'unlimited' } })).toBe(false)
  })
})
