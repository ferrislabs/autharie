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
import type { AgentPolicy } from '../../types'

interface Props {
  policy: AgentPolicy | null
  isDeleting: boolean
  error?: string
  onConfirm: () => void
  onCancel: () => void
}

export function DeletePolicyDialog({ policy, isDeleting, error, onConfirm, onCancel }: Props) {
  return (
    <Dialog open={policy !== null} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete {policy?.name}?</DialogTitle>
          <DialogDescription>
            The policy is removed from this deployment’s realm. Calls it allowed are denied from then
            on unless another policy permits them.
          </DialogDescription>
        </DialogHeader>
        {error && (
          <p role='alert' className='text-sm text-destructive'>
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant='outline' onClick={onCancel}>
            Cancel
          </Button>
          <Button variant='destructive' disabled={isDeleting} onClick={onConfirm}>
            {isDeleting && <Spinner />}
            Delete policy
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
