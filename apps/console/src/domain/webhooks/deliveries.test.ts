import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { canRetry, deliveryBadge, relativeTime, truncate } from './deliveries'

describe('delivery status', () => {
  it('maps each status to a label and tone', () => {
    expect(deliveryBadge('succeeded')).toEqual({ label: 'Succeeded', tone: 'success' })
    expect(deliveryBadge('failed')).toEqual({ label: 'Failed', tone: 'danger' })
    expect(deliveryBadge('delivering')).toEqual({ label: 'Delivering', tone: 'progress' })
    expect(deliveryBadge('pending')).toEqual({ label: 'Pending', tone: 'neutral' })
  })

  it('offers retry on failed deliveries only', () => {
    const of = (status: Schemas.DeliveryStatus) => ({ status }) as Schemas.WebhookDelivery
    expect(canRetry(of('failed'))).toBe(true)
    expect(canRetry(of('succeeded'))).toBe(false)
    expect(canRetry(of('pending'))).toBe(false)
  })
})

describe('text and time', () => {
  it('truncates with an ellipsis, leaving short text alone', () => {
    expect(truncate('abcdef', 4)).toBe('abc…')
    expect(truncate('abc', 4)).toBe('abc')
  })

  const now = Date.parse('2026-01-01T12:00:00Z')
  it('says how long ago', () => {
    expect(relativeTime('2026-01-01T11:59:50Z', now)).toBe('just now')
    expect(relativeTime('2026-01-01T11:55:00Z', now)).toBe('5 minutes ago')
    expect(relativeTime('2026-01-01T11:59:00Z', now)).toBe('1 minute ago')
    expect(relativeTime('2026-01-01T09:00:00Z', now)).toBe('3 hours ago')
    expect(relativeTime('2025-12-30T12:00:00Z', now)).toBe('2 days ago')
  })

  it('does not say the future', () => {
    expect(relativeTime('2026-01-01T12:05:00Z', now)).toBe('just now')
  })
})
