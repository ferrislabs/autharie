import { useId, useMemo } from 'react'
import { Area, AreaChart, CartesianGrid, Tooltip, XAxis, YAxis } from 'recharts'
import { ArrowDownRight, ArrowUpRight, Minus } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Skeleton } from '@/components/ui/skeleton'
import { totalOf } from './traffic'

export type TrafficMetric = 'requests' | 'logins' | 'active_users' | 'token_events'
export type TrafficRange = '24h' | '7d' | '30d'

export interface TrafficSeries {
  metric: TrafficMetric
  /** One point per bucket, oldest first. `null` is a bucket nothing reported. */
  points: { at: number; value: number | null }[]
  /** The same total over the period just before, for the change shown under it. */
  previousTotal: number | null
}

const METRICS: { key: TrafficMetric; label: string; noun: string }[] = [
  { key: 'requests', label: 'Requests', noun: 'requests' },
  { key: 'logins', label: 'Sign-ins', noun: 'sign-ins' },
  { key: 'active_users', label: 'Active users', noun: 'active users' },
  { key: 'token_events', label: 'Tokens issued', noun: 'tokens' },
]

const RANGES: { key: TrafficRange; label: string }[] = [
  { key: '24h', label: '24h' },
  { key: '7d', label: '7d' },
  { key: '30d', label: '30d' },
]

const compact = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 })
const full = new Intl.NumberFormat('en')

function change(current: number | null, previous: number | null): number | null {
  if (current === null || previous === null || previous === 0) return null
  return (current - previous) / previous
}

function Delta({ ratio }: { ratio: number | null }) {
  if (ratio === null) return <span className='text-xs text-muted-foreground'>No earlier data</span>

  const flat = Math.abs(ratio) < 0.005
  const Icon = flat ? Minus : ratio > 0 ? ArrowUpRight : ArrowDownRight

  return (
    <span
      className={cn(
        'inline-flex items-center gap-0.5 text-xs font-medium tabular-nums',
        flat ? 'text-muted-foreground' : ratio > 0 ? 'text-emerald-600 dark:text-emerald-400' : 'text-rose-600 dark:text-rose-400',
      )}
    >
      <Icon className='h-3.5 w-3.5' />
      {flat ? 'No change' : `${Math.abs(ratio * 100).toFixed(ratio > -0.1 && ratio < 0.1 ? 1 : 0)}%`}
    </span>
  )
}

function tickFormatter(range: TrafficRange) {
  const time = new Intl.DateTimeFormat('en', { hour: '2-digit', minute: '2-digit', hour12: false })
  const day = new Intl.DateTimeFormat('en', { day: 'numeric', month: 'short' })
  return (at: number) => (range === '24h' ? time.format(at) : day.format(at))
}

function tooltipFormatter(range: TrafficRange) {
  const format = new Intl.DateTimeFormat('en', {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
    ...(range === '30d' ? {} : { hour: '2-digit', minute: '2-digit', hour12: false }),
  })
  return (at: number) => format.format(at)
}

interface Props {
  series: TrafficSeries[]
  metric: TrafficMetric
  range: TrafficRange
  onMetricChange: (metric: TrafficMetric) => void
  onRangeChange: (range: TrafficRange) => void
  loading?: boolean
}

export function TrafficPanel({ series, metric, range, onMetricChange, onRangeChange, loading }: Props) {
  const gradientId = useId()
  const selected = series.find((entry) => entry.metric === metric)
  const noun = METRICS.find((entry) => entry.key === metric)?.noun ?? metric
  const formatTick = useMemo(() => tickFormatter(range), [range])
  const formatInstant = useMemo(() => tooltipFormatter(range), [range])
  const hasData = selected?.points.some((point) => point.value !== null) ?? false

  return (
    <section className='rounded-lg border bg-card' aria-label='Traffic'>
      <div className='flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3'>
        <h2 className='text-sm font-semibold'>Traffic</h2>
        <div role='radiogroup' aria-label='Period' className='flex rounded-md border p-0.5'>
          {RANGES.map(({ key, label }) => (
            <button
              key={key}
              type='button'
              role='radio'
              aria-checked={range === key}
              onClick={() => onRangeChange(key)}
              className={cn(
                'rounded-[4px] px-2.5 py-1 text-xs font-medium tabular-nums transition-colors',
                range === key
                  ? 'bg-secondary text-foreground'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div role='tablist' aria-label='Metric' className='grid grid-cols-2 border-b sm:grid-cols-4'>
        {METRICS.map(({ key, label }, index) => {
          const entry = series.find((candidate) => candidate.metric === key)
          const value = entry ? totalOf(entry.points, key) : null
          const active = key === metric

          return (
            <button
              key={key}
              type='button'
              role='tab'
              aria-selected={active}
              onClick={() => onMetricChange(key)}
              className={cn(
                'relative flex flex-col items-start gap-1 px-4 py-3 text-left transition-colors',
                index % 2 === 1 && 'border-l',
                index === 2 && 'sm:border-l',
                index > 1 && 'border-t sm:border-t-0',
                active ? 'bg-muted/40' : 'hover:bg-muted/20',
              )}
            >
              {active && <span className='absolute inset-x-0 top-0 h-0.5 bg-primary' />}
              <span className='text-xs text-muted-foreground'>
                {label}
                {key === 'active_users' && <span className='sr-only'> (peak)</span>}
              </span>
              <span
                className='text-xl font-semibold tracking-tight'
                title={value === null ? undefined : full.format(value)}
              >
                {value === null ? '—' : compact.format(value)}
              </span>
              <Delta ratio={change(value, entry?.previousTotal ?? null)} />
            </button>
          )
        })}
      </div>

      <div className='px-2 pt-4 pb-2'>
        {loading ? (
          <Skeleton className='h-60 w-full' />
        ) : hasData && selected ? (
          <AreaChart
            responsive
            data={selected.points}
            margin={{ top: 4, right: 12, bottom: 0, left: 0 }}
            style={{ width: '100%', height: 240 }}
            accessibilityLayer
          >
            <defs>
              <linearGradient id={gradientId} x1='0' y1='0' x2='0' y2='1'>
                <stop offset='0%' stopColor='var(--color-primary)' stopOpacity={0.28} />
                <stop offset='100%' stopColor='var(--color-primary)' stopOpacity={0} />
              </linearGradient>
            </defs>
            <CartesianGrid vertical={false} stroke='var(--color-border)' />
            <XAxis
              dataKey='at'
              type='number'
              scale='time'
              domain={['dataMin', 'dataMax']}
              tickFormatter={formatTick}
              tickLine={false}
              axisLine={false}
              tickMargin={8}
              minTickGap={48}
              tick={{ fill: 'var(--color-muted-foreground)', fontSize: 11 }}
            />
            <YAxis
              tickFormatter={(value: number) => compact.format(value)}
              tickLine={false}
              axisLine={false}
              width={44}
              allowDecimals={false}
              tick={{ fill: 'var(--color-muted-foreground)', fontSize: 11 }}
            />
            <Tooltip
              cursor={{ stroke: 'var(--color-muted-foreground)', strokeWidth: 1 }}
              content={({ active, payload }) => {
                const point = payload?.[0]?.payload as TrafficSeries['points'][number] | undefined
                if (!active || !point) return null
                return (
                  <div className='rounded-md border bg-popover px-3 py-2 text-xs shadow-lg shadow-black/20'>
                    <p className='text-muted-foreground'>{formatInstant(point.at)}</p>
                    <p className='mt-0.5 font-medium tabular-nums'>
                      {point.value === null ? 'Nothing reported' : `${full.format(point.value)} ${noun}`}
                    </p>
                  </div>
                )
              }}
            />
            <Area
              type='monotone'
              dataKey='value'
              stroke='var(--color-primary)'
              strokeWidth={2}
              fill={`url(#${gradientId})`}
              connectNulls={false}
              isAnimationActive={false}
              activeDot={{ r: 4, strokeWidth: 2, stroke: 'var(--color-card)', fill: 'var(--color-primary)' }}
            />
          </AreaChart>
        ) : (
          <div className='flex h-60 flex-col items-center justify-center gap-1 text-center'>
            <p className='text-sm font-medium'>No {noun} yet</p>
            <p className='max-w-xs text-xs text-muted-foreground'>
              Traffic appears here a minute after the first request reaches this instance.
            </p>
          </div>
        )}
      </div>
    </section>
  )
}
