import { describe, expect, it } from 'vitest'
import { UNLIMITED, limited } from '@/domain/quotas/quota'
import { describeLimit } from './limit'

describe('the webhook limit', () => {
  it('shows how many are used', () => {
    expect(describeLimit(3, limited(5))).toEqual({ label: '3 of 5', atLimit: false, reason: null })
  })

  it('blocks creating at the limit, and says what to do', () => {
    const limit = describeLimit(5, limited(5))
    expect(limit.atLimit).toBe(true)
    expect(limit.reason).toMatch(/5 webhooks/)
    expect(limit.reason).toMatch(/delete one|higher quota/i)
  })

  it('follows a raised quota instead of the plan', () => {
    expect(describeLimit(5, limited(8))).toEqual({ label: '5 of 8', atLimit: false, reason: null })
  })

  it('blocks everything under a quota of zero', () => {
    expect(describeLimit(0, limited(0)).atLimit).toBe(true)
  })

  it('never blocks when nothing limits it', () => {
    expect(describeLimit(500, UNLIMITED)).toEqual({ label: '500', atLimit: false, reason: null })
  })

  it('does not guess the limit before the quota is known', () => {
    expect(describeLimit(3, undefined)).toEqual({ label: '3', atLimit: false, reason: null })
  })
})
