import { useState } from 'react'
import { ArrowLeft, Check, Copy } from 'lucide-react'
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
import { cn } from '@/lib/utils'
import type { Schemas } from '@/api/api.client'
import { afterSubmit, canRegister, emptyDraft, toRegisterRequest } from '../../credentials'
import { PROVIDERS, providerById } from '../../providers'
import type { Provider } from '../../types/cloud-provider'
import { AnimatedHeight } from './animated-height'
import { stepWidth, type ConnectStep } from './connect-step'
import { ProviderCard, ProviderLogoTile } from './provider-card'

interface FormProps {
  provider: Provider
  isSaving: boolean
  refusal?: string
  onBack: () => void
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

function Step({
  number,
  title,
  last = false,
  children,
}: {
  number: number
  title: string
  last?: boolean
  children: React.ReactNode
}) {
  return (
    <li className='relative flex gap-4'>
      {!last && <span aria-hidden='true' className='absolute bottom-[-1.5rem] left-3.5 top-8 w-px bg-border' />}
      <span
        aria-hidden='true'
        className='flex size-7 shrink-0 items-center justify-center rounded-full border bg-background text-xs font-semibold'
      >
        {number}
      </span>
      <div className='min-w-0 flex-1 space-y-3'>
        <h3 className='pt-1 text-sm font-semibold'>
          <span className='sr-only'>{number}. </span>
          {title}
        </h3>
        {children}
      </div>
    </li>
  )
}

export function CredentialForm({ provider, isSaving, refusal, onBack, onSubmit }: FormProps) {
  const definition = providerById(provider)
  const [draft, setDraft] = useState(emptyDraft(provider))

  return (
    <form
      className='space-y-6'
      onSubmit={(event) => {
        event.preventDefault()
        if (!canRegister(draft) || isSaving) return
        const request = toRegisterRequest(draft)
        setDraft(afterSubmit(draft))
        onSubmit(request)
      }}
    >
      <DialogHeader>
        <div className='flex items-center gap-3'>
          <ProviderLogoTile provider={provider} />
          <div className='space-y-1'>
            <DialogTitle>Connect {definition.label}</DialogTitle>
            <DialogDescription>
              Follow these steps and give Autharie access to your {definition.label} account.
            </DialogDescription>
          </div>
        </div>
      </DialogHeader>

      <ol className='space-y-6'>
        <Step number={1} title='Create an API key'>
          <p className='text-sm text-muted-foreground'>
            In the {definition.label} console, create an API key for the project your clusters
            should be created in. Give its IAM application or user these permission sets:
          </p>
          <PermissionList names={definition.permissionSets} />
          <p className='text-xs text-muted-foreground'>
            IAMReadOnly is only needed so Autharie can check the key&apos;s permissions. Do not
            grant any other permission: Autharie refuses a key that holds more than needed.
          </p>
        </Step>

        <Step number={2} title='Fill these information' last>
          <div className='grid gap-4 sm:grid-cols-2'>
            {definition.fields.map((field) => (
              <div key={field.id} className={cn('space-y-2', field.wide && 'sm:col-span-2')}>
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
          </div>
        </Step>
      </ol>

      {refusal && (
        <p role='alert' className='text-sm text-destructive'>
          {refusal}
        </p>
      )}

      <DialogFooter>
        <Button type='button' variant='ghost' onClick={onBack}>
          <ArrowLeft className='h-4 w-4' />
          Back
        </Button>
        <Button type='submit' disabled={!canRegister(draft) || isSaving}>
          {isSaving && <Spinner />}
          Connect
        </Button>
      </DialogFooter>
    </form>
  )
}

export function ProviderPicker({ onSelect }: { onSelect: (provider: Provider) => void }) {
  return (
    <div className='space-y-6'>
      <DialogHeader>
        <DialogTitle>Connect a provider</DialogTitle>
        <DialogDescription>
          Choose the cloud provider your clusters are created in.
        </DialogDescription>
      </DialogHeader>
      <div className='grid gap-3 sm:grid-cols-2'>
        {PROVIDERS.map((provider) => (
          <ProviderCard key={provider.id} provider={provider.id} onSelect={onSelect} />
        ))}
      </div>
    </div>
  )
}

interface Props {
  step: ConnectStep | null
  onStep: (step: ConnectStep) => void
  onClose: () => void
  isSaving: boolean
  refusal?: string
  onSubmit: (request: Schemas.RegisterCloudCredentialRequest) => void
}

export function ConnectProviderDialog({ step, onStep, onClose, isSaving, refusal, onSubmit }: Props) {
  const provider = step?.provider ?? null

  return (
    <Dialog open={step !== null} onOpenChange={(next) => !next && onClose()}>
      <DialogContent
        className={cn(
          'max-h-[90vh] overflow-y-auto transition-[max-width] duration-300 ease-out motion-reduce:transition-none',
          stepWidth(step ?? { provider: null }),
        )}
      >
        <AnimatedHeight>
          {provider ? (
            <CredentialForm
              key={provider}
              provider={provider}
              isSaving={isSaving}
              refusal={refusal}
              onBack={() => onStep({ provider: null })}
              onSubmit={onSubmit}
            />
          ) : (
            <ProviderPicker onSelect={(next) => onStep({ provider: next })} />
          )}
        </AnimatedHeight>
      </DialogContent>
    </Dialog>
  )
}
