import type { AvailabilityDayResponse } from '@/api/deployment-availability.api'
import type { AvailabilityDay } from './availability-history'

const DAY = 86_400_000
const SHOWN_DAYS = 90

export function toAvailabilityDays(days: AvailabilityDayResponse[]): AvailabilityDay[] {
  return days.map((entry) => ({
    day: Date.parse(`${entry.day}T00:00:00Z`),
    uptime: entry.uptime,
    downMinutes: Math.round(entry.down_seconds / 60),
  }))
}

export function unobservedDays(now: number): AvailabilityDay[] {
  const today = Math.floor(now / DAY) * DAY
  return Array.from({ length: SHOWN_DAYS }, (_, index) => ({
    day: today - (SHOWN_DAYS - 1 - index) * DAY,
    uptime: null,
    downMinutes: 0,
  }))
}

export function probeEvery(seconds: number): string {
  if (seconds === 60) return 'every minute'
  if (seconds < 60) return `every ${seconds} seconds`
  const minutes = Math.round(seconds / 60)
  return `every ${minutes} minutes`
}
