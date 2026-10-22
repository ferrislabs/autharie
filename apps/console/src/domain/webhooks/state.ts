import type { Schemas } from '@/api/api.client'
import type { Tone } from '@/components/ui/status-badge'

export interface WebhookBadge {
  label: string
  tone: Tone
  reason: string | null
}

export function webhookBadge(state: Schemas.WebhookState): WebhookBadge {
  switch (state.phase) {
    case 'Pending':
      return { label: 'Applying', tone: 'progress', reason: null }
    case 'Active':
      return { label: 'Active', tone: 'success', reason: null }
    case 'Failed':
      return { label: 'Failed', tone: 'danger', reason: state.reason }
  }
}
