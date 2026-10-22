import { useState } from 'react'
import { ArrowLeft, ArrowRight } from 'lucide-react'
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
import { ConnectStepper } from './connect-stepper'
import { stepWidth, type ConnectStep } from './connect-step'
import { emphasised } from './emphasis'
import { ExternalLinkButton, GuideStepView } from './guide-step'
import { ProviderCard, ProviderLogoTile } from './provider-card'
import { nextStep, previousStep } from './wizard'

interface FormProps {
  provider: Provider
  isSaving: boolean
  refusal?: string
  initialStep?: number
  onBack: () => void
  onSubmit: (request: Schemas.RegisterCloudCredentialRequest) => void
}

export function CredentialForm({
  provider,
  isSaving,
  refusal,
  initialStep = 0,
  onBack,
  onSubmit,
}: FormProps) {
  const definition = providerById(provider)
  const [draft, setDraft] = useState(emptyDraft(provider))
  const [current, setCurrent] = useState(initialStep)
  const labels = [...definition.guide.map((step) => step.title), definition.connect.title]
  const last = labels.length - 1
  const guide = definition.guide[current]

  return (
    <form
      className='space-y-6'
      onSubmit={(event) => {
        event.preventDefault()
        if (current !== last || !canRegister(draft) || isSaving) return
        const request = toRegisterRequest(draft)
        setDraft(afterSubmit(draft))
        onSubmit(request)
      }}
    >
      <Button
        type='button'
        variant='ghost'
        size='sm'
        className='-ml-2'
        onClick={() => (current === 0 ? onBack() : setCurrent(previousStep(current)))}
      >
        <ArrowLeft className='h-4 w-4' />
        Back
      </Button>

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

      <ConnectStepper labels={labels} current={current} onGo={setCurrent} />

      {guide ? (
        <GuideStepView step={guide} groups={definition.permissionGroups} />
      ) : (
        <section className='space-y-4'>
          <div className='space-y-1'>
            <h3 className='text-base font-semibold'>{definition.connect.heading}</h3>
            <p className='text-sm text-muted-foreground'>{definition.connect.intro}</p>
          </div>
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
                {field.hint && (
                  <p className='text-xs text-muted-foreground'>{emphasised(field.hint)}</p>
                )}
                {field.link && <ExternalLinkButton link={field.link} />}
              </div>
            ))}
          </div>
        </section>
      )}

      {refusal && current === last && (
        <p role='alert' className='text-sm text-destructive'>
          {refusal}
        </p>
      )}

      <DialogFooter>
        {current === last ? (
          <Button key='connect' type='submit' disabled={!canRegister(draft) || isSaving}>
            {isSaving && <Spinner />}
            Connect
          </Button>
        ) : (
          <Button key='next' type='button' onClick={() => setCurrent(nextStep(current, last))}>
            Next
            <ArrowRight className='h-4 w-4' />
          </Button>
        )}
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
