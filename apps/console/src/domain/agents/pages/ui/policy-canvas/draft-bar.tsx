import { AlertTriangle, Loader2, Upload, Undo2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { Pending } from '../../../canvas-model'
import type { Progress } from './use-canvas-draft'

export function DraftBar({
  pending,
  progress,
  error,
  serverChanged,
  onDiscard,
  onPublish,
  onFocusRule,
}: {
  pending: Pending
  progress: Progress | null
  error: string | null
  serverChanged: boolean
  onDiscard: () => void
  onPublish: () => void
  onFocusRule: (ruleId: string) => void
}) {
  const count = pending.count
  const busy = progress !== null
  const blocked = pending.blocking.length > 0

  return (
    <div className='sticky top-0 z-20 space-y-2 rounded-lg border bg-card px-4 py-2.5 shadow-xs'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <p className='flex items-center gap-2 text-sm' aria-live='polite'>
          {busy ? (
            <>
              <Loader2 className='size-4 animate-spin text-muted-foreground' />
              Publishing {Math.min(progress.done + 1, progress.total)} of {progress.total}…
            </>
          ) : count > 0 ? (
            <>
              <span className='size-2 rounded-full bg-primary' aria-hidden />
              <span className='font-medium'>
                {count} unpublished {count === 1 ? 'change' : 'changes'}
              </span>
            </>
          ) : (
            <span className='text-muted-foreground'>Everything is published.</span>
          )}
        </p>
        <div className='flex items-center gap-2'>
          <Button type='button' variant='outline' size='sm' disabled={count === 0 || busy} onClick={onDiscard}>
            <Undo2 className='size-4' />
            Discard
          </Button>
          <Button type='button' size='sm' disabled={count === 0 || busy || blocked} onClick={onPublish}>
            <Upload className='size-4' />
            Publish
          </Button>
        </div>
      </div>

      {blocked && (
        <div className='space-y-1 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200'>
          <p className='font-medium'>Fix these rules before publishing</p>
          <ul className='space-y-0.5'>
            {pending.blocking.map((entry) => (
              <li key={entry.ruleId}>
                <button
                  type='button'
                  className='flex items-start gap-1.5 text-left underline-offset-2 hover:underline'
                  onClick={() => onFocusRule(entry.ruleId)}
                >
                  <AlertTriangle className='mt-0.5 size-3 shrink-0' />
                  <span>
                    <span className='font-medium'>{entry.name.trim() || 'Unnamed rule'}</span>
                    {' — '}
                    {entry.issues[0]}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {serverChanged && (
        <p className='text-xs text-muted-foreground'>
          The published rules changed while you were editing. Publishing will overwrite them; discard to start from
          the latest.
        </p>
      )}

      {error && (
        <p role='alert' className='text-xs text-destructive'>
          {error}
        </p>
      )}
    </div>
  )
}
