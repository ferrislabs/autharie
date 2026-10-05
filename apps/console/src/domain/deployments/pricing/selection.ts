import { normalise, type Selection } from './model'

export interface Change {
  selection: Selection
  movedToBusiness: boolean
}

export function applyChange(current: Selection, patch: Partial<Selection>): Change {
  const wanted = { ...current, ...patch }
  const selection = normalise(wanted)

  return { selection, movedToBusiness: selection.plan !== wanted.plan }
}

export const STARTER_MOVED_NOTICE =
  'Starter stops at 25,000 accounts, so this deployment moved up to Business.'
