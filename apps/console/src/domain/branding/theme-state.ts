import type { Schemas } from '@/api/api.client'
import type { Tone } from '@/components/ui/status-badge'

export interface ThemeStateBadge {
  label: string
  tone: Tone
  message: string | null
}

export const ACTIVE_BADGE: { label: string; tone: Tone } = { label: 'Active', tone: 'accent' }

export function themeStateBadge(state: Schemas.ThemeSync): ThemeStateBadge {
  switch (state.phase) {
    case 'Pending':
      return { label: 'Applying', tone: 'progress', message: null }
    case 'Applied':
      return { label: 'Applied', tone: 'success', message: null }
    case 'Failed':
      return { label: 'Failed', tone: 'danger', message: state.message }
  }
}

export function isApplying(state: Schemas.ThemeSync | null | undefined): boolean {
  return state?.phase === 'Pending'
}

export function themeStateSummary(activeName: string | null): string {
  return activeName === null ? 'FerrisKey’s default look' : `Active theme: ${activeName}`
}
