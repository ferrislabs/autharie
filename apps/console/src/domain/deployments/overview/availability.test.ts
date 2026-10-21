import { describe, expect, it } from 'vitest'
import { probeEvery, toAvailabilityDays, unobservedDays } from './availability'

describe('toAvailabilityDays', () => {
  it('maps a day to UTC midnight and whole minutes of downtime', () => {
    const [day] = toAvailabilityDays([
      { day: '2026-10-21', checks: 1440, failed: 3, uptime: 0.9979, down_seconds: 190 },
    ])

    expect(day).toEqual({ day: Date.UTC(2026, 9, 21), uptime: 0.9979, downMinutes: 3 })
  })

  it('keeps a day nobody checked as null, never as zero', () => {
    const [day] = toAvailabilityDays([
      { day: '2026-10-20', checks: 0, failed: 0, uptime: null, down_seconds: 0 },
    ])

    expect(day.uptime).toBeNull()
  })
})

describe('unobservedDays', () => {
  it('gives ninety days ending today, none of them observed', () => {
    const days = unobservedDays(Date.UTC(2026, 9, 22, 15, 30))

    expect(days).toHaveLength(90)
    expect(days[89].day).toBe(Date.UTC(2026, 9, 22))
    expect(days[0].day).toBe(Date.UTC(2026, 9, 22) - 89 * 86_400_000)
    expect(days.every((day) => day.uptime === null && day.downMinutes === 0)).toBe(true)
  })
})

describe('probeEvery', () => {
  it('says every minute for sixty seconds', () => {
    expect(probeEvery(60)).toBe('every minute')
  })

  it('counts minutes and seconds otherwise', () => {
    expect(probeEvery(300)).toBe('every 5 minutes')
    expect(probeEvery(30)).toBe('every 30 seconds')
  })
})
