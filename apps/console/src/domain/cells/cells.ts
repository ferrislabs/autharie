import type { Schemas } from '@/api/api.client'
import type { Tone } from '@/components/ui/status-badge'

export const NO_OWNER = '__none__'

const LABELS: Record<Schemas.CellStatus, string> = {
  provisioning: 'Provisioning',
  open: 'Open',
  full: 'Full',
  draining: 'Draining',
  retired: 'Retired',
  failed: 'Failed',
}

const TONES: Record<Schemas.CellStatus, Tone> = {
  provisioning: 'progress',
  open: 'success',
  full: 'warning',
  draining: 'warning',
  retired: 'neutral',
  failed: 'danger',
}

export function cellStatusLabel(status: Schemas.CellStatus): string {
  return LABELS[status]
}

export function cellStatusTone(status: Schemas.CellStatus): Tone {
  return TONES[status]
}

export function canDrain(status: Schemas.CellStatus): boolean {
  return status === 'open' || status === 'full'
}

export function canRetire(cell: Pick<Schemas.CellView, 'status' | 'realms'>): boolean {
  const retirable =
    cell.status === 'draining' || cell.status === 'failed' || cell.status === 'provisioning'

  return retirable && cell.realms === 0
}

export function occupancy(cell: Pick<Schemas.CellView, 'capacity' | 'realms'>): number {
  if (cell.capacity <= 0) return 0

  return Math.min(100, Math.round((cell.realms / cell.capacity) * 100))
}

export function liveCells(cells: Pick<Schemas.CellView, 'status'>[]): number {
  return cells.filter((cell) => cell.status !== 'retired').length
}

export function ownerFromChoice(choice: string): string | null {
  return choice === NO_OWNER ? null : choice
}

export function choiceFromOwner(owner: string | null | undefined): string {
  return owner ?? NO_OWNER
}

export function regionToOpen(typed: string): string | undefined {
  const region = typed.trim()

  return region.length > 0 ? region : undefined
}
