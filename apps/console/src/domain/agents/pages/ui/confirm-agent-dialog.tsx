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
import { confirmCopy, type AgentAction } from '../../agent-actions'

interface Props {
  action: AgentAction | null
  isBusy: boolean
  error?: string
  onConfirm: () => void
  onCancel: () => void
}

export function ConfirmAgentDialog({ action, isBusy, error, onConfirm, onCancel }: Props) {
  const copy = action ? confirmCopy(action) : null

  return (
    <Dialog open={action !== null} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{copy?.title}</DialogTitle>
          <DialogDescription>{copy?.body}</DialogDescription>
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
          <Button variant='destructive' disabled={isBusy} onClick={onConfirm}>
            {isBusy && <Spinner />}
            {copy?.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
