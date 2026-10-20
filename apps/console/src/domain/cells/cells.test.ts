import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  NO_OWNER,
  canDrain,
  canRetire,
  cellStatusLabel,
  cellStatusTone,
  choiceFromOwner,
  liveCells,
  occupancy,
  ownerFromChoice,
  regionToOpen,
} from './cells'

function cell(overrides: Partial<Schemas.CellView> = {}): Schemas.CellView {
  return {
    id: 'cell',
    region: 'fr-par',
    data_plane_id: 'dp',
    instance_deployment_id: 'instance',
    status: 'open',
    capacity: 200,
    realms: 0,
    created_at: '2026-10-20T00:00:00Z',
    ...overrides,
  }
}

describe('what an operator may do with a cell', () => {
  it('drains a cell that takes tenants, open or full, and no other', () => {
    expect(canDrain('open')).toBe(true)
    expect(canDrain('full')).toBe(true)
    expect(canDrain('provisioning')).toBe(false)
    expect(canDrain('draining')).toBe(false)
    expect(canDrain('retired')).toBe(false)
    expect(canDrain('failed')).toBe(false)
  })

  it('retires a cell only once it holds no realm', () => {
    expect(canRetire(cell({ status: 'draining', realms: 0 }))).toBe(true)
    expect(canRetire(cell({ status: 'draining', realms: 3 }))).toBe(false)
    expect(canRetire(cell({ status: 'failed', realms: 0 }))).toBe(true)
    expect(canRetire(cell({ status: 'provisioning', realms: 0 }))).toBe(true)
  })

  it('does not retire a cell that still takes tenants or is already retired', () => {
    expect(canRetire(cell({ status: 'open' }))).toBe(false)
    expect(canRetire(cell({ status: 'full', realms: 200 }))).toBe(false)
    expect(canRetire(cell({ status: 'retired' }))).toBe(false)
  })
})

describe('how a cell is drawn', () => {
  it('names every status', () => {
    const statuses: Schemas.CellStatus[] = [
      'provisioning',
      'open',
      'full',
      'draining',
      'retired',
      'failed',
    ]

    for (const status of statuses) {
      expect(cellStatusLabel(status).length).toBeGreaterThan(0)
      expect(cellStatusTone(status)).toBeTruthy()
    }
    expect(cellStatusTone('failed')).toBe('danger')
    expect(cellStatusTone('open')).toBe('success')
  })

  it('measures occupancy against capacity, capped at the whole', () => {
    expect(occupancy(cell({ capacity: 200, realms: 50 }))).toBe(25)
    expect(occupancy(cell({ capacity: 200, realms: 0 }))).toBe(0)
    expect(occupancy(cell({ capacity: 10, realms: 12 }))).toBe(100)
    expect(occupancy(cell({ capacity: 0, realms: 0 }))).toBe(0)
  })
})

describe('the owner of the cells', () => {
  it('counts the cells that keep the owner in place', () => {
    const cells = [
      cell({ status: 'retired' }),
      cell({ status: 'open' }),
      cell({ status: 'failed' }),
    ]

    expect(liveCells(cells)).toBe(2)
    expect(liveCells([cell({ status: 'retired' })])).toBe(0)
    expect(liveCells([])).toBe(0)
  })

  it('turns the choice in the select into the value sent, and back', () => {
    expect(ownerFromChoice(NO_OWNER)).toBeNull()
    expect(ownerFromChoice('org-1')).toBe('org-1')
    expect(choiceFromOwner(null)).toBe(NO_OWNER)
    expect(choiceFromOwner(undefined)).toBe(NO_OWNER)
    expect(choiceFromOwner('org-1')).toBe('org-1')
  })
})

describe('the region of a new cell', () => {
  it('trims what was typed', () => {
    expect(regionToOpen('  fr-par ')).toBe('fr-par')
  })

  it('refuses nothing as a region', () => {
    expect(regionToOpen('   ')).toBeUndefined()
    expect(regionToOpen('')).toBeUndefined()
  })
})
