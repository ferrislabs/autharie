import { formatDistanceToNowStrict } from 'date-fns'
import {
  ArrowUpCircle,
  CircleAlert,
  DatabaseBackup,
  History,
  Mail,
  Network,
  Palette,
  Play,
  Rocket,
  RotateCcw,
  Settings2,
  Square,
  Trash2,
} from 'lucide-react'
import { StatusBadge } from '@/components/ui/status-badge'
import { cn } from '@/lib/utils'

export type ActivityOutcome = 'done' | 'running' | 'failed'

export interface ActivityEntry {
  id: string
  type: string
  at: number
  outcome: ActivityOutcome
  /** A short fact about this one occurrence, such as the versions of an upgrade. */
  detail?: string
}

/**
 * The words a person would use. An action type is the platform's name for the
 * work; the feed is for someone checking what happened to their instance.
 */
const DESCRIPTIONS: Record<string, { title: string; icon: React.ComponentType<{ className?: string }> }> = {
  'deployment.create': { title: 'Instance created', icon: Rocket },
  'deployment.update': { title: 'Settings changed', icon: Settings2 },
  'deployment.upgrade': { title: 'Upgraded', icon: ArrowUpCircle },
  'deployment.network_access': { title: 'Network access changed', icon: Network },
  'deployment.iam_settings': { title: 'Branding updated', icon: Palette },
  'deployment.portal_theme': { title: 'Login theme updated', icon: Palette },
  'deployment.smtp': { title: 'Email provider updated', icon: Mail },
  'deployment.backup': { title: 'Backup taken', icon: DatabaseBackup },
  'deployment.restore': { title: 'Restored from a backup', icon: RotateCcw },
  'deployment.drill': { title: 'Restore drill', icon: History },
  'deployment.stop': { title: 'Stopped', icon: Square },
  'deployment.start': { title: 'Started', icon: Play },
  'deployment.delete': { title: 'Deletion requested', icon: Trash2 },
  'deployment.unreachable': { title: 'Instance unreachable', icon: CircleAlert },
}

function describe(type: string) {
  return DESCRIPTIONS[type] ?? { title: type, icon: Settings2 }
}

const absolute = new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' })

export function ActivityFeed({ entries, onViewAll }: { entries: ActivityEntry[]; onViewAll?: () => void }) {
  return (
    <section className='flex flex-col rounded-lg border bg-card' aria-label='Activity'>
      <div className='flex items-center justify-between border-b px-4 py-3'>
        <h2 className='text-sm font-semibold'>Activity</h2>
        {onViewAll && (
          <button
            type='button'
            onClick={onViewAll}
            className='text-xs text-muted-foreground transition-colors hover:text-foreground'
          >
            View all
          </button>
        )}
      </div>

      {entries.length === 0 ? (
        <p className='px-4 py-10 text-center text-sm text-muted-foreground'>
          Changes to this instance appear here as they happen.
        </p>
      ) : (
        <ol className='relative px-4 py-2'>
          {entries.map((entry, index) => {
            const { title, icon: Icon } = describe(entry.type)
            const last = index === entries.length - 1
            return (
              <li key={entry.id} className='relative flex gap-3 py-2.5'>
                {!last && (
                  <span aria-hidden className='absolute top-9 bottom-0 left-[13px] w-px bg-border' />
                )}
                <span
                  className={cn(
                    'relative z-10 flex h-7 w-7 shrink-0 items-center justify-center rounded-full border bg-background',
                    entry.outcome === 'failed' && 'border-[#d03b3b]/50 text-[#d03b3b]',
                    entry.outcome !== 'failed' && 'text-muted-foreground',
                  )}
                >
                  <Icon className='h-3.5 w-3.5' />
                </span>
                <div className='min-w-0 flex-1'>
                  <div className='flex items-start justify-between gap-2'>
                    <p className='text-sm leading-7 font-medium'>{title}</p>
                    <time
                      dateTime={new Date(entry.at).toISOString()}
                      title={absolute.format(entry.at)}
                      className='shrink-0 text-xs leading-7 text-muted-foreground tabular-nums'
                    >
                      {formatDistanceToNowStrict(entry.at, { addSuffix: true })}
                    </time>
                  </div>
                  {(entry.detail || entry.outcome !== 'done') && (
                    <div className='flex flex-wrap items-center gap-2'>
                      {entry.detail && <p className='text-xs text-muted-foreground'>{entry.detail}</p>}
                      {entry.outcome === 'running' && <StatusBadge tone='progress'>In progress</StatusBadge>}
                      {entry.outcome === 'failed' && <StatusBadge tone='danger'>Failed</StatusBadge>}
                    </div>
                  )}
                </div>
              </li>
            )
          })}
        </ol>
      )}
    </section>
  )
}
