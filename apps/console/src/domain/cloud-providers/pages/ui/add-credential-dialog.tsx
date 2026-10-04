import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import {
  SCALEWAY_SECRET_HELP,
  afterSubmit,
  canRegister,
  secretProblem,
  toRegisterRequest,
} from '../../credentials'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  isSaving: boolean
  refusal?: string
  onSubmit: (label: string, secret: string) => void
}

export function AddCredentialDialog({ open, onOpenChange, isSaving, refusal, onSubmit }: Props) {
  const [draft, setDraft] = useState(afterSubmit())
  const problem = secretProblem(draft.secret)

  const close = (next: boolean) => {
    if (!next) setDraft(afterSubmit())
    onOpenChange(next)
  }

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <form
          className='space-y-4'
          onSubmit={(event) => {
            event.preventDefault()
            if (!canRegister(draft)) return
            const request = toRegisterRequest(draft)
            setDraft(afterSubmit())
            onSubmit(request.label, request.secret)
          }}
        >
          <DialogHeader>
            <DialogTitle>Add a cloud account</DialogTitle>
            <DialogDescription>
              Clusters are created in this account and billed to it by the provider.
            </DialogDescription>
          </DialogHeader>

          <div className='space-y-2'>
            <Label htmlFor='credential-label'>Label</Label>
            <Input
              id='credential-label'
              value={draft.label}
              onChange={(event) => setDraft({ ...draft, label: event.target.value })}
              placeholder='Scaleway production'
              autoComplete='off'
              required
            />
          </div>

          <div className='space-y-2'>
            <Label htmlFor='credential-secret'>Scaleway API key (JSON)</Label>
            <Input
              id='credential-secret'
              type='password'
              value={draft.secret}
              onChange={(event) => setDraft({ ...draft, secret: event.target.value })}
              autoComplete='off'
              spellCheck={false}
              aria-invalid={problem !== null}
              required
            />
            <p className='text-xs text-muted-foreground'>{SCALEWAY_SECRET_HELP}</p>
            {problem && <p className='text-xs text-destructive'>{problem}</p>}
          </div>

          {refusal && (
            <p role='alert' className='text-sm text-destructive'>
              {refusal}
            </p>
          )}

          <DialogFooter>
            <Button type='button' variant='ghost' onClick={() => close(false)}>
              Cancel
            </Button>
            <Button type='submit' disabled={!canRegister(draft) || isSaving}>
              {isSaving && <Spinner />}
              Add account
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
