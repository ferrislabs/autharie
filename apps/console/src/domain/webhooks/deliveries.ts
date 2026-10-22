import type { Schemas } from '@/api/api.client'
import type { Tone } from '@/components/ui/status-badge'

export const DELIVERY_FILTERS: { value: Schemas.DeliveryStatus | 'all'; label: string }[] = [
  { value: 'all', label: 'All statuses' },
  { value: 'succeeded', label: 'Succeeded' },
  { value: 'failed', label: 'Failed' },
  { value: 'delivering', label: 'Delivering' },
  { value: 'pending', label: 'Pending' },
]

export const DELIVERIES_REFRESH_MS = 10_000

const BADGES: Record<Schemas.DeliveryStatus, { label: string; tone: Tone }> = {
  succeeded: { label: 'Succeeded', tone: 'success' },
  failed: { label: 'Failed', tone: 'danger' },
  delivering: { label: 'Delivering', tone: 'progress' },
  pending: { label: 'Pending', tone: 'neutral' },
}

export function deliveryBadge(status: Schemas.DeliveryStatus) {
  return BADGES[status]
}

export function canRetry(delivery: Schemas.WebhookDelivery): boolean {
  return delivery.status === 'failed'
}

export function truncate(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`
}

const UNITS: [number, string][] = [
  [86_400_000, 'day'],
  [3_600_000, 'hour'],
  [60_000, 'minute'],
]

export function relativeTime(iso: string, now: number): string {
  const elapsed = now - Date.parse(iso)

  for (const [size, unit] of UNITS) {
    if (elapsed >= size) {
      const count = Math.floor(elapsed / size)
      return `${count} ${unit}${count === 1 ? '' : 's'} ago`
    }
  }
  return 'just now'
}
