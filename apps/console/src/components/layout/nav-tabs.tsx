import { Link, useRouterState } from '@tanstack/react-router'
import { StatusBadge } from '@/components/ui/status-badge'
import { cn } from '@/lib/utils'
import { isUnder } from '@/lib/paths'

export interface Tab {
  label: string
  to: string
  icon: React.ComponentType<{ className?: string }>
  exact?: boolean
  /** Announced but not there yet: shown with a badge and not a link. */
  soon?: boolean
}

export function NavTabs({ tabs }: { tabs: Tab[] }) {
  const pathname = useRouterState({ select: (state) => state.location.pathname })

  return (
    <nav className='flex items-center gap-1 overflow-x-auto px-4 [scrollbar-width:none]'>
      {tabs.map(({ label, to, icon: Icon, exact, soon }) => {
        if (soon) {
          return (
            <span
              key={label}
              aria-disabled='true'
              className='-mb-px flex shrink-0 cursor-not-allowed items-center gap-2 border-b-2 border-transparent px-3 py-2.5 text-sm text-muted-foreground opacity-60'
            >
              <Icon className='h-4 w-4' />
              {label}
              <StatusBadge tone='neutral' dot={false}>
                Soon
              </StatusBadge>
            </span>
          )
        }

        const active = isUnder(pathname, to, exact)

        return (
          <Link
            key={to}
            to={to}
            className={cn(
              '-mb-px flex shrink-0 items-center gap-2 border-b-2 px-3 py-2.5 text-sm transition-colors',
              active
                ? 'border-primary font-medium text-primary'
                : 'border-transparent text-muted-foreground hover:text-foreground',
            )}
          >
            <Icon className='h-4 w-4' />
            {label}
          </Link>
        )
      })}
    </nav>
  )
}
