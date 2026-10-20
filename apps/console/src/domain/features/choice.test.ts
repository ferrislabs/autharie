import { describe, expect, it } from 'vitest'
import { OVERRIDE_CHOICES, choiceOf, isFeatureEnabled, overrideOf, stateLabel } from './choice'

describe('choiceOf', () => {
  it('reads true and false as an override', () => {
    expect(choiceOf(true)).toBe('on')
    expect(choiceOf(false)).toBe('off')
  })

  it('reads null and absent as following the default', () => {
    expect(choiceOf(null)).toBe('default')
    expect(choiceOf(undefined)).toBe('default')
  })
})

describe('overrideOf', () => {
  it('sends null to clear the override', () => {
    expect(overrideOf('default')).toBeNull()
  })

  it('sends a boolean for an explicit choice', () => {
    expect(overrideOf('on')).toBe(true)
    expect(overrideOf('off')).toBe(false)
  })

  it('is the inverse of choiceOf for every choice', () => {
    for (const choice of OVERRIDE_CHOICES) {
      expect(choiceOf(overrideOf(choice))).toBe(choice)
    }
  })
})

describe('isFeatureEnabled', () => {
  it('is on when the key is listed', () => {
    expect(isFeatureEnabled(['customer_cloud'], 'customer_cloud')).toBe(true)
  })

  it('is off when the key is not listed', () => {
    expect(isFeatureEnabled([], 'customer_cloud')).toBe(false)
  })

  it('is off while the answer is not loaded', () => {
    expect(isFeatureEnabled(undefined, 'customer_cloud')).toBe(false)
    expect(isFeatureEnabled(null, 'customer_cloud')).toBe(false)
  })
})

describe('stateLabel', () => {
  it('names both states', () => {
    expect(stateLabel(true)).toBe('On')
    expect(stateLabel(false)).toBe('Off')
  })
})
