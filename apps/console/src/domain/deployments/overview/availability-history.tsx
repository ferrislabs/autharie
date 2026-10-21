import { CircleCheck, CircleDashed, CircleX, TriangleAlert } from 'lucide-react'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { probeEvery } from './availability'

export interface AvailabilityDay {
  /** Midnight UTC of the day. */
  day: number
  /** Share of the day's checks that reached the instance, 0 to 1. `null`: no checks. */
  uptime: number | null
  downMinutes: number
}

type DayState = 'operational' | 'degraded' | 'down' | 'none'

/**
 * Fixed status colours, never themed: they mean the same thing on every
 * screen, and each always travels with an icon and a word.
 */
const STATES: Record<
  DayState,
  { label: string; bar: string; icon: React.ComponentType<{ className?: string }>; iconClass: string }
> = {
  operational: { label: 'Operational', bar: 'bg-[#0ca30c]', icon: CircleCheck, iconClass: 'text-[#0ca30c]' },
  degraded: { label: 'Degraded', bar: 'bg-[#fab219]', icon: TriangleAlert, iconClass: 'text-[#fab219]' },
  down: { label: 'Down', bar: 'bg-[#d03b3b]', icon: CircleX, iconClass: 'text-[#d03b3b]' },
  none: { label: 'No data', bar: 'bg-muted', icon: CircleDashed, iconClass: 'text-muted-foreground' },
}

function stateOf(day: AvailabilityDay): DayState {
  if (day.uptime === null) return 'none'
  if (day.uptime >= 0.999) return 'operational'
  if (day.uptime >= 0.95) return 'degraded'
  return 'down'
}

function percent(uptime: number): string {
  const value = uptime * 100
  // 99.99 and 100 must not both round to "100%": the last decimals are the
  // whole point of an availability figure.
  return `${value >= 99.995 ? '100' : value.toFixed(value >= 99 ? 2 : 1)}%`
}

function duration(minutes: number): string {
  if (minutes < 60) return `${minutes} min`
  const hours = Math.floor(minutes / 60)
  const rest = minutes % 60
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`
}

const dayFormat = new Intl.DateTimeFormat('en', { weekday: 'short', day: 'numeric', month: 'short', timeZone: 'UTC' })

interface Props {
  days: AvailabilityDay[]
  uptime: number | null
  probeIntervalSeconds: number
}

export function AvailabilityHistory({ days, uptime, probeIntervalSeconds }: Props) {
  const incidents = days.filter((day) => stateOf(day) === 'degraded' || stateOf(day) === 'down').length
  const downtime = days.reduce((sum, day) => sum + day.downMinutes, 0)

  return (
    <section className='rounded-lg border bg-card' aria-label='Availability'>
      <div className='flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1 border-b px-4 py-3'>
        <h2 className='text-sm font-semibold'>Availability</h2>
        <p className='text-xs text-muted-foreground'>Checked {probeEvery(probeIntervalSeconds)} from the control plane</p>
      </div>

      <div className='space-y-5 p-4'>
        <dl className='grid grid-cols-3 gap-4'>
          <div>
            <dt className='text-xs text-muted-foreground'>Uptime, 90 days</dt>
            <dd className='mt-1 text-xl font-semibold tracking-tight'>
              {uptime === null ? '—' : percent(uptime)}
            </dd>
          </div>
          <div>
            <dt className='text-xs text-muted-foreground'>Days with incidents</dt>
            <dd className='mt-1 text-xl font-semibold tracking-tight'>{incidents}</dd>
          </div>
          <div>
            <dt className='text-xs text-muted-foreground'>Total downtime</dt>
            <dd className='mt-1 text-xl font-semibold tracking-tight'>
              {uptime === null ? '—' : downtime === 0 ? 'None' : duration(downtime)}
            </dd>
          </div>
        </dl>

        <div>
          <div className='flex h-9 items-stretch gap-[2px]' role='list' aria-label='Daily availability'>
            {days.map((day, index) => {
              const state = stateOf(day)
              // Ninety bars on a phone are a pixel each: the last thirty are
              // what fits there and still reads as days.
              const olderThanAMonth = index < days.length - 30
              const { label, bar, icon: Icon, iconClass } = STATES[state]
              return (
                <Tooltip key={day.day}>
                  <TooltipTrigger asChild>
                    <span
                      role='listitem'
                      tabIndex={0}
                      aria-label={`${dayFormat.format(day.day)}: ${label}`}
                      className={cn(
                        'min-w-0 flex-1 rounded-[2px] outline-none transition-opacity hover:opacity-70 focus-visible:ring-2 focus-visible:ring-ring',
                        olderThanAMonth && 'hidden sm:block',
                        bar,
                      )}
                    />
                  </TooltipTrigger>
                  <TooltipContent side='top' className='text-xs'>
                    <p className='font-medium'>{dayFormat.format(day.day)}</p>
                    <p className='mt-1 flex items-center gap-1.5'>
                      <Icon className={cn('h-3.5 w-3.5', iconClass)} />
                      {label}
                      {day.uptime !== null && <span className='tabular-nums'>· {percent(day.uptime)}</span>}
                    </p>
                    {day.downMinutes > 0 && (
                      <p className='mt-0.5 text-muted-foreground'>Down for {duration(day.downMinutes)}</p>
                    )}
                  </TooltipContent>
                </Tooltip>
              )
            })}
          </div>
          <div className='mt-2 flex justify-between text-[11px] text-muted-foreground'>
            <span className='hidden sm:inline'>90 days ago</span>
            <span className='sm:hidden'>30 days ago</span>
            <span>Today</span>
          </div>
        </div>

        <ul className='flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground'>
          {(Object.keys(STATES) as DayState[]).map((state) => {
            const { label, icon: Icon, iconClass } = STATES[state]
            return (
              <li key={state} className='flex items-center gap-1.5'>
                <Icon className={cn('h-3.5 w-3.5', iconClass)} />
                {label}
              </li>
            )
          })}
        </ul>
      </div>
    </section>
  )
}
