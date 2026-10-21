import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { estimateRows } from './estimate-rows'

const line = (deployment_id: string, amount: number): Schemas.StatementLineResponse => ({
  deployment_id,
  tier: 'business',
  monthly_price: 22500,
  starts_at: '2026-10-01T00:00:00Z',
  ends_at: '2026-10-21T00:00:00Z',
  covered_seconds: 1000,
  amount,
})

describe('the estimate rows', () => {
  it('sum the lines of one deployment', () => {
    const rows = estimateRows([line('a', 100), line('b', 40), line('a', 25)], new Map())
    expect(rows).toEqual([
      { id: 'a', name: 'Unknown deployment (a)', amount: 125 },
      { id: 'b', name: 'Unknown deployment (b)', amount: 40 },
    ])
  })

  it('name a deployment when it is known', () => {
    const rows = estimateRows([line('a', 1), line('b', 2)], new Map([['b', 'Bravo']]))
    expect(rows.map((row) => row.name)).toEqual(['Bravo', 'Unknown deployment (a)'])
  })

  it('say a deployment is unknown, with the start of its id, when the list does not hold it', () => {
    const rows = estimateRows(
      [line('0b6e3a1c-9d4f-4c7a-8e21-5f3d2a9b7c10', 1)],
      new Map([['other', 'Other']]),
    )
    expect(rows[0].name).toBe('Unknown deployment (0b6e3a1c)')
    expect(rows[0].name).not.toContain('9d4f')
  })

  it('order the rows by name', () => {
    const rows = estimateRows(
      [line('a', 1), line('b', 2)],
      new Map([
        ['a', 'Zulu'],
        ['b', 'Alpha'],
      ]),
    )
    expect(rows.map((row) => row.name)).toEqual(['Alpha', 'Zulu'])
  })

  it('are none when there is no line', () => {
    expect(estimateRows([], new Map())).toEqual([])
  })
})
