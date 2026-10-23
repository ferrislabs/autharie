import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Spinner } from '@/components/ui/spinner'
import type { AgentPolicy, AgentServer } from '../../types'

interface Props {
  server: AgentServer | null
  referring: AgentPolicy[]
  isRemoving: boolean
  error?: string
  onConfirm: () => void
  onCancel: () => void
}

export function ReferringPolicies({ policies, slug }: { policies: AgentPolicy[]; slug: string }) {
  if (policies.length === 0) return null
  const one = policies.length === 1

  return (
    <div
      role='note'
      className='space-y-2 rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'
    >
      <p>
        {one ? 'This policy refers' : 'These policies refer'} to {slug}. {one ? 'It stays' : 'They stay'} and{' '}
        {one ? 'shows' : 'show'} an unknown server until you change {one ? 'it' : 'them'}.
      </p>
      <ul className='list-disc pl-5'>
        {policies.map((policy) => (
          <li key={policy.id}>{policy.name}</li>
        ))}
      </ul>
    </div>
  )
}

export function RemoveServerDialog({ server, referring, isRemoving, error, onConfirm, onCancel }: Props) {
  return (
    <Dialog open={server !== null} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Remove {server?.name}?</DialogTitle>
          <DialogDescription>
            Agents can no longer reach it: its gateway path, route and audience client are removed.
            Nothing is deleted on the server itself.
          </DialogDescription>
        </DialogHeader>
        <ReferringPolicies policies={referring} slug={server?.slug ?? ''} />
        {error && (
          <p role='alert' className='text-sm text-destructive'>
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant='outline' onClick={onCancel}>
            Cancel
          </Button>
          <Button variant='destructive' disabled={isRemoving} onClick={onConfirm}>
            {isRemoving && <Spinner />}
            Remove server
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
