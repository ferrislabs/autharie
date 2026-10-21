import { describe, expect, it } from 'vitest'
import { previousWindow, rangeSpec, seriesFor, totalOf, windowFor } from './traffic'

const NOW = Date.parse('2026-10-22T12:00:00Z')
const MINUTE = 60_000

describe('rangeSpec', () => {
  it('buckets 24h by 15 minutes, 7d by 2 hours and 30d by a day', () => {
    expect(rangeSpec('24h')).toMatchObject({ minutes: 1440, slots: 96 })
    expect(rangeSpec('7d')).toMatchObject({ minutes: 10080, slots: 84 })
    expect(rangeSpec('30d')).toMatchObject({ minutes: 43200, slots: 30 })
  })
})

describe('windows', () => {
  it('ends the window now and reaches back by the range', () => {
    expect(windowFor('24h', NOW)).toEqual({ from: NOW - 1440 * MINUTE, until: NOW })
  })

  it('puts the previous window right before this one, of the same length', () => {
    const current = windowFor('7d', NOW)
    const before = previousWindow(current)

    expect(before.until).toBe(current.from)
    expect(before.until - before.from).toBe(current.until - current.from)
  })
})

describe('totalOf', () => {
  const points = [
    { at: 1, value: 4 },
    { at: 2, value: null },
    { at: 3, value: 9 },
  ]

  it('adds a count and ignores holes', () => {
    expect(totalOf(points, 'requests')).toBe(13)
  })

  it('takes the peak of active users rather than their sum', () => {
    expect(totalOf(points, 'active_users')).toBe(9)
  })

  it('is null when nothing was reported, not zero', () => {
    expect(totalOf([{ at: 1, value: null }], 'logins')).toBeNull()
  })
})

describe('seriesFor', () => {
  const window = windowFor('24h', NOW)

  it('leaves a bucket nothing reported as a hole', () => {
    const series = seriesFor(
      'requests',
      [{ bucket: new Date(window.from + 3 * MINUTE).toISOString(), value: 5 }],
      window,
      '24h',
      undefined,
    )

    expect(series.points).toHaveLength(96)
    expect(series.points[0].value).toBe(5)
    expect(series.points[1].value).toBeNull()
  })

  it('compares with the previous window when it was fetched', () => {
    const before = previousWindow(window)
    const series = seriesFor(
      'requests',
      [],
      window,
      '24h',
      [{ bucket: new Date(before.from + MINUTE).toISOString(), value: 7 }],
    )

    expect(series.previousTotal).toBe(7)
  })

  it('has no earlier total when the previous window was not fetched or was empty', () => {
    expect(seriesFor('logins', [], window, '24h', undefined).previousTotal).toBeNull()
    expect(seriesFor('logins', [], window, '24h', []).previousTotal).toBeNull()
  })
})
