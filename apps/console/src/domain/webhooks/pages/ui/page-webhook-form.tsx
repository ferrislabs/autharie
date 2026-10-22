import { useState } from 'react'
import { ArrowLeft } from 'lucide-react'
import { SectionPage } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import type { EventInfo } from '../../events'
import { checkForm, isReadyToSave, type WebhookForm } from '../../form'
import { EventPicker } from './event-picker'
import { HeaderEditor } from './header-editor'

interface Props {
  mode: 'create' | 'edit'
  isLoading: boolean
  initial: WebhookForm
  realms: string[]
  catalogue: EventInfo[]
  isSaving: boolean
  error?: string
  onSubmit: (form: WebhookForm) => void
  onCancel: () => void
}

export function PageWebhookForm({ isLoading, ...props }: Props) {
  if (isLoading) return <Skeleton className='h-96 w-full' />

  return <FormBody {...props} />
}

function FormBody({
  mode,
  initial,
  realms,
  catalogue,
  isSaving,
  error,
  onSubmit,
  onCancel,
}: Omit<Props, 'isLoading'>) {
  const [form, setForm] = useState<WebhookForm>(initial)
  const [touched, setTouched] = useState(false)
  const problems = checkForm(form)
  const shown = touched ? problems : {}
  const realmChoices =
    realms.includes(form.realm) || form.realm === '' ? realms : [form.realm, ...realms]
  const endpointProblem = form.endpoint.trim() === '' && !touched ? undefined : problems.endpoint

  return (
    <SectionPage
      title={mode === 'create' ? 'New webhook' : 'Edit webhook'}
      description='Choose which events to send and where to send them.'
      actions={
        <Button variant='ghost' size='sm' onClick={onCancel}>
          <ArrowLeft className='h-3.5 w-3.5' />
          Back
        </Button>
      }
    >
      <form
        className='max-w-3xl space-y-6'
        onSubmit={(event) => {
          event.preventDefault()
          setTouched(true)
          if (isReadyToSave(form) && !isSaving) onSubmit(form)
        }}
        noValidate
      >
        {error && (
          <p
            role='alert'
            className='rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive'
          >
            {error}
          </p>
        )}

        <div className='grid gap-4 sm:grid-cols-2'>
          <Field label='Realm' htmlFor='webhook-realm' problem={shown.realm}>
            <Select value={form.realm} onValueChange={(realm) => setForm({ ...form, realm })}>
              <SelectTrigger id='webhook-realm' aria-invalid={!!shown.realm}>
                <SelectValue placeholder='Choose a realm' />
              </SelectTrigger>
              <SelectContent>
                {realmChoices.map((realm) => (
                  <SelectItem key={realm} value={realm}>
                    {realm}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <Field label='Name' htmlFor='webhook-name' problem={shown.name}>
            <Input
              id='webhook-name'
              value={form.name}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
              placeholder='Sync users to billing'
              aria-invalid={!!shown.name}
            />
          </Field>
        </div>

        <Field label='Description' htmlFor='webhook-description' hint='Optional. Shown only here.'>
          <Input
            id='webhook-description'
            value={form.description}
            onChange={(event) => setForm({ ...form, description: event.target.value })}
          />
        </Field>

        <Field
          label='Endpoint'
          htmlFor='webhook-endpoint'
          problem={endpointProblem}
          hint='Where events are sent. Must start with https://.'
        >
          <Input
            id='webhook-endpoint'
            type='url'
            value={form.endpoint}
            onChange={(event) => setForm({ ...form, endpoint: event.target.value })}
            placeholder='https://example.com/hooks/autharie'
            aria-invalid={!!endpointProblem}
            className='font-mono'
          />
        </Field>

        <Field label='Events' problem={shown.events}>
          <EventPicker
            catalogue={catalogue}
            selected={form.events}
            onChange={(events) => setForm({ ...form, events })}
            invalid={!!shown.events}
          />
        </Field>

        <Field
          label='Custom headers'
          problem={shown.headers}
          hint='Sent with every delivery. Values are stored encrypted and never shown again.'
        >
          <HeaderEditor rows={form.headers} onChange={(headers) => setForm({ ...form, headers })} />
        </Field>

        <div className='flex items-center gap-2 border-t pt-4'>
          <Button type='submit' disabled={isSaving}>
            {isSaving && <Spinner className='size-3.5' />}
            {isSaving ? 'Saving' : mode === 'create' ? 'Create webhook' : 'Save changes'}
          </Button>
          <Button type='button' variant='outline' onClick={onCancel} disabled={isSaving}>
            Cancel
          </Button>
        </div>
      </form>
    </SectionPage>
  )
}

function Field({
  label,
  htmlFor,
  problem,
  hint,
  children,
}: {
  label: string
  htmlFor?: string
  problem?: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div className='space-y-1.5'>
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {problem ? (
        <p className='text-xs text-destructive'>{problem}</p>
      ) : (
        hint && <p className='text-xs text-muted-foreground'>{hint}</p>
      )}
    </div>
  )
}
