import { describe, expect, it } from 'vitest'
import { nextStep, previousStep, stepStatus } from './wizard'

describe('the steps of the wizard', () => {
  it('marks what is behind as done, here as current, ahead as to do', () => {
    expect([0, 1, 2, 3].map((index) => stepStatus(index, 2))).toEqual([
      'done',
      'done',
      'current',
      'todo',
    ])
  })

  it('goes back one step, and stops at the first', () => {
    expect(previousStep(2)).toBe(1)
    expect(previousStep(0)).toBe(0)
  })

  it('goes forward one step, and stops at the last', () => {
    expect(nextStep(1, 3)).toBe(2)
    expect(nextStep(3, 3)).toBe(3)
  })
})
