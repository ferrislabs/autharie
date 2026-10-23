import type { Schemas } from '@/api/api.client'
import type { Tone } from '@/components/ui/status-badge'

export interface SmtpBadge {
  label: string
  tone: Tone
  message: string | null
}

export function smtpBadge(state: Schemas.SmtpState): SmtpBadge {
  switch (state.phase) {
    case 'Pending':
      return { label: 'Applying', tone: 'progress', message: null }
    case 'Applied':
      return { label: 'Applied', tone: 'success', message: null }
    case 'Failed':
      return { label: 'Failed', tone: 'danger', message: state.message }
  }
}

export function isPending(state: Schemas.SmtpState | undefined): boolean {
  return state?.phase === 'Pending'
}

export function isNotFerriskey(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'status' in error &&
    (error as { status: unknown }).status === 409
  )
}
