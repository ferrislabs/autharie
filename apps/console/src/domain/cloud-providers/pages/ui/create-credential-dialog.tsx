import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
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
import type { Schemas } from '@/api/api.client'
import { afterSubmit, canRegister, emptyDraft, toRegisterRequest } from '../../credentials'
import { providerById } from '../../providers'
import type { Provider } from '../../types/cloud-provider'

interface FormProps {
  provider: Provider
  isSaving: boolean
  refusal?: string
  onCancel: () => void
  onSubmit: (request: Schemas.RegisterCloudCredentialRequest) => void
}

function PermissionList({ names }: { names: string[] }) {
  const [copied, setCopied] = useState(false)

  return (
    <div className='rounded-md border bg-muted/40 p-3'>
      <div className='flex items-start justify-between gap-2'>
        <ul className='space-y-1 font-mono text-xs'>
          {names.map((name) => (
            <li key={name}>{name}</li>
          ))}
        </ul>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          aria-label='Copy the permission sets'
          onClick={() => {
            void navigator.clipboard.writeText(names.join('\n')).then(() => setCopied(true))
          }}
        >
          {copied ? <Check className='h-4 w-4' /> : <Copy className='h-4 w-4' />}
        </Button>
      </div>
    </div>
  )
}

export function CredentialForm({ provider, isSaving, refusal, onCancel, onSubmit }: FormProps) {
  const definition = providerById(provider)
  const [draft, setDraft] = useState(emptyDraft(provider))

  return (
    <form
      className='space-y-4'
      onSubmit={(event) => {
        event.preventDefault()
        if (!canRegister(draft) || isSaving) return
        const request = toRegisterRequest(draft)
        setDraft(afterSubmit(draft))
        onSubmit(request)
      }}
    >
      <DialogHeader>
        <DialogTitle>Create new credential</DialogTitle>
        <DialogDescription>
          Follow these steps and give Autharie access to your {definition.label} account.
        </DialogDescription>
      </DialogHeader>

      <section className='space-y-3 rounded-lg border p-4'>
        <h3 className='text-sm font-semibold'>1. Create an API key</h3>
        <p className='text-sm text-muted-foreground'>
          In the {definition.label} console, create an API key for the project your clusters should
          be created in. Give its IAM application or user these permission sets:
        </p>
        <PermissionList names={definition.permissionSets} />
        <p className='text-xs text-muted-foreground'>
          IAMReadOnly is only needed so Autharie can check the key&apos;s permissions. Do not grant
          any other permission: Autharie refuses a key that holds more than needed.
        </p>
      </section>

      <section className='space-y-3 rounded-lg border p-4'>
        <h3 className='text-sm font-semibold'>2. Fill these information</h3>
        {definition.fields.map((field) => (
          <div key={field.id} className='space-y-2'>
            <Label htmlFor={`credential-${field.id}`}>{field.label}</Label>
            <Input
              id={`credential-${field.id}`}
              type={field.masked ? 'password' : 'text'}
              value={draft.values[field.id] ?? ''}
              onChange={(event) =>
                setDraft({
                  ...draft,
                  values: { ...draft.values, [field.id]: event.target.value },
                })
              }
              placeholder={field.placeholder}
              autoComplete='off'
              spellCheck={false}
              required
            />
          </div>
        ))}
      </section>

      {refusal && (
        <p role='alert' className='text-sm text-destructive'>
          {refusal}
        </p>
      )}

      <DialogFooter>
        <Button type='button' variant='ghost' onClick={onCancel}>
          Cancel
        </Button>
        <Button type='submit' disabled={!canRegister(draft) || isSaving}>
          {isSaving && <Spinner />}
          Create
        </Button>
      </DialogFooter>
    </form>
  )
}

interface Props {
  provider: Provider | null
  onClose: () => void
  isSaving: boolean
  refusal?: string
  onSubmit: (request: Schemas.RegisterCloudCredentialRequest) => void
}

export function CreateCredentialDialog({ provider, onClose, isSaving, refusal, onSubmit }: Props) {
  return (
    <Dialog open={provider !== null} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className='max-h-[90vh] overflow-y-auto sm:max-w-xl'>
        {provider && (
          <CredentialForm
            key={provider}
            provider={provider}
            isSaving={isSaving}
            refusal={refusal}
            onCancel={onClose}
            onSubmit={onSubmit}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}
