import { Check } from 'lucide-react'
import { cn } from '@/lib/utils'
import { stepStatus } from './wizard'

interface Props {
  labels: string[]
  current: number
  onGo: (index: number) => void
}

export function ConnectStepper({ labels, current, onGo }: Props) {
  return (
    <nav aria-label='Steps'>
      <ol className='flex items-center'>
        {labels.map((label, index) => {
          const status = stepStatus(index, current)

          return (
            <li
              key={label}
              className={cn('flex items-center', index < labels.length - 1 && 'flex-1')}
            >
              <button
                type='button'
                disabled={status === 'todo'}
                aria-current={status === 'current' ? 'step' : undefined}
                onClick={() => onGo(index)}
                className='group flex items-center gap-2 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default'
              >
                <span
                  aria-hidden='true'
                  className={cn(
                    'flex size-7 shrink-0 items-center justify-center rounded-full border text-xs font-semibold transition-colors',
                    status === 'current' && 'border-primary bg-primary text-primary-foreground',
                    status === 'done' && 'border-primary bg-primary/10 text-primary',
                    status === 'todo' && 'text-muted-foreground',
                  )}
                >
                  {status === 'done' ? <Check className='size-3.5' /> : index + 1}
                </span>
                <span
                  className={cn(
                    'hidden text-sm font-medium sm:inline',
                    status === 'todo' && 'text-muted-foreground',
                  )}
                >
                  {label}
                </span>
              </button>
              {index < labels.length - 1 && (
                <span
                  aria-hidden='true'
                  className={cn(
                    'mx-3 h-px flex-1 transition-colors',
                    index < current ? 'bg-primary' : 'bg-border',
                  )}
                />
              )}
            </li>
          )
        })}
      </ol>
    </nav>
  )
}
