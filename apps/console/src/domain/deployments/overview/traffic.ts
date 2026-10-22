import { aggregationFor, toSeries, type Bucket } from '@/domain/usage/series'
import type { TrafficMetric, TrafficRange, TrafficSeries } from './traffic-panel'

interface RangeSpec {
  minutes: number
  slots: number
}

const SPECS: Record<TrafficRange, RangeSpec> = {
  '24h': { minutes: 24 * 60, slots: 96 },
  '7d': { minutes: 7 * 24 * 60, slots: 84 },
  '30d': { minutes: 30 * 24 * 60, slots: 30 },
}

export interface Window {
  from: number
  until: number
}

export function rangeSpec(range: TrafficRange): RangeSpec {
  return SPECS[range]
}

export function windowFor(range: TrafficRange, now: number): Window {
  return { from: now - SPECS[range].minutes * 60_000, until: now }
}

export function previousWindow(window: Window): Window {
  return { from: window.from - (window.until - window.from), until: window.from }
}

export function totalOf(points: TrafficSeries['points'], metric: TrafficMetric): number | null {
  const values = points.map((point) => point.value).filter((value): value is number => value !== null)
  if (values.length === 0) return null

  return metric === 'active_users'
    ? Math.max(...values)
    : values.reduce((sum, value) => sum + value, 0)
}

export function seriesFor(
  metric: TrafficMetric,
  buckets: Bucket[],
  window: Window,
  range: TrafficRange,
  previous: Bucket[] | undefined,
): TrafficSeries {
  const { slots } = SPECS[range]
  const aggregation = aggregationFor(metric)
  const earlier = previousWindow(window)

  return {
    metric,
    points: toSeries(buckets, window.from, window.until, slots, aggregation),
    previousTotal: previous
      ? totalOf(toSeries(previous, earlier.from, earlier.until, slots, aggregation), metric)
      : null,
  }
}
