import { RefreshCw } from 'lucide-react'
import { EmptyState } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'

export const SLOW_NOTICE = 'This is taking longer than usual. The data plane is still working on it.'

interface Props {
  what: string
  isLoading: boolean
  isSlow: boolean
  error?: string
  onRetry: () => void
}

export function RequestNotice({ what, isLoading, isSlow, error, onRetry }: Props) {
  if (error !== undefined) {
    return (
      <div
        role='alert'
        className='flex items-center justify-between gap-4 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive'
      >
        <span>{error}</span>
        <Button variant='outline' size='sm' onClick={onRetry}>
          <RefreshCw className='h-4 w-4' />
          Try again
        </Button>
      </div>
    )
  }

  if (isLoading) {
    return (
      <EmptyState
        icon={<Spinner />}
        title={`Loading ${what}…`}
        description={isSlow ? SLOW_NOTICE : undefined}
      />
    )
  }

  return null
}
