import { describe, expect, it } from 'vitest'
import { webhookBadge } from './state'

describe('the state badge', () => {
  it('calls pending "Applying" and marks it busy', () => {
    expect(webhookBadge({ phase: 'Pending' })).toEqual({
      label: 'Applying',
      tone: 'progress',
      reason: null,
    })
  })

  it('calls active "Active"', () => {
    expect(webhookBadge({ phase: 'Active', ferriskey_id: 'f' })).toEqual({
      label: 'Active',
      tone: 'success',
      reason: null,
    })
  })

  it('keeps the reason a webhook failed', () => {
    expect(webhookBadge({ phase: 'Failed', reason: 'realm gone' })).toEqual({
      label: 'Failed',
      tone: 'danger',
      reason: 'realm gone',
    })
  })
})
