import { describe, expect, it } from 'vitest'
import { hasAgents } from './agents-feature'

describe('the agents feature', () => {
  it('is on only when listed', () => {
    expect(hasAgents(['agents'])).toBe(true)
    expect(hasAgents(['customer_cloud'])).toBe(false)
  })

  it('is off while the list is unknown', () => {
    expect(hasAgents(undefined)).toBe(false)
    expect(hasAgents(null)).toBe(false)
  })
})
