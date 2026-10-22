import { describe, expect, it } from 'vitest'
import { describeLimit, maxWebhooks } from './limit'

describe('the webhook limit', () => {
  it('is 5 on Free and 25 on paid plans', () => {
    expect(maxWebhooks('Free')).toBe(5)
    expect(maxWebhooks('Starter')).toBe(25)
    expect(maxWebhooks('Business')).toBe(25)
    expect(maxWebhooks('Enterprise')).toBe(25)
  })

  it('shows how many are used', () => {
    expect(describeLimit(3, 'Free')).toEqual({ label: '3 of 5', atLimit: false, reason: null })
  })

  it('blocks creating at the limit, and says what to do', () => {
    const limit = describeLimit(5, 'Free')
    expect(limit.atLimit).toBe(true)
    expect(limit.reason).toMatch(/5 webhooks/)
    expect(limit.reason).toMatch(/delete one|upgrade/i)
  })

  it('does not guess the limit before the plan is known', () => {
    expect(describeLimit(3, undefined)).toEqual({ label: '3', atLimit: false, reason: null })
  })
})
