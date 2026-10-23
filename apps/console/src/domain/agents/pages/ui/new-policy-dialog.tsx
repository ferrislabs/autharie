import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { PolicyTemplateDraft } from '../../policy-templates'
import { TemplateCards } from './template-cards'

interface Props {
  open: boolean
  onPick: (draft: PolicyTemplateDraft | undefined) => void
  onClose: () => void
}

export function NewPolicyDialog({ open, onPick, onClose }: Props) {
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className='sm:max-w-2xl'>
        <DialogHeader>
          <DialogTitle>New policy</DialogTitle>
          <DialogDescription>
            Start from a template and adjust it, or write a policy from scratch.
          </DialogDescription>
        </DialogHeader>
        <TemplateCards onPick={onPick} />
        <DialogFooter>
          <Button variant='outline' onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => onPick(undefined)}>Start blank</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
