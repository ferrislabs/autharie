import type { OfferChoice } from './types'

export const OFFER_CHOICES: readonly OfferChoice[] = ['default', 'open', 'closed']

export const OFFER_CHOICE_LABELS: Record<OfferChoice, string> = {
  default: 'Default',
  open: 'Open',
  closed: 'Closed',
}

export function offerChoiceOf(override: boolean | null | undefined): OfferChoice {
  if (override === true) return 'open'
  if (override === false) return 'closed'
  return 'default'
}

export function offerOverrideOf(choice: OfferChoice): boolean | null {
  if (choice === 'open') return true
  if (choice === 'closed') return false
  return null
}

export function openLabel(open: boolean): 'Open' | 'Closed' {
  return open ? 'Open' : 'Closed'
}
