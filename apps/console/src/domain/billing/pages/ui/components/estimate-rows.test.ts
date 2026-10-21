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
      { id: 'a', name: 'a', amount: 125 },
      { id: 'b', name: 'b', amount: 40 },
    ])
  })

  it('name a deployment when it is known, and fall back to its id otherwise', () => {
    const rows = estimateRows([line('a', 1), line('b', 2)], new Map([['b', 'Bravo']]))
    expect(rows.map((row) => row.name)).toEqual(['a', 'Bravo'])
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
