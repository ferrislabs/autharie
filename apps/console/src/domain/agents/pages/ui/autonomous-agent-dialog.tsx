import { useState } from 'react'
import { AlertTriangle } from 'lucide-react'
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
import { Spinner } from '@/components/ui/spinner'
import type { FlowEvent, Revealed } from '../../credential-flow'
import { curlSnippet, n8nFields } from '../../credential-snippets'
import { displayName } from '../../registry'
import type { AgentClient } from '../../types'
import { CopyButton } from './copy-button'
import { Field, TEXTAREA_CLASS } from './field'

export type AutonomousMode = { kind: 'create' } | { kind: 'rotate'; client: AgentClient }

interface Props {
  mode: AutonomousMode | null
  event: FlowEvent | null
  onCreate: (name: string, description: string) => void
  onRotate: () => void
  onClose: () => void
}

function Secret({ label, value }: { label: string; value: string }) {
  return (
    <div className='space-y-1'>
      <div className='text-xs font-medium text-muted-foreground'>{label}</div>
      <div className='flex items-center gap-2'>
        <code className='min-w-0 flex-1 break-all rounded-md border bg-muted px-2.5 py-1.5 font-mono text-xs'>
          {value}
        </code>
        <CopyButton text={value} label={`Copy ${label.toLowerCase()}`} />
      </div>
    </div>
  )
}

function Revealing({ revealed }: { revealed: Revealed }) {
  const call = {
    tokenUrl: revealed.tokenUrl,
    clientId: revealed.clientId,
    secret: revealed.secret,
    audience: revealed.audience,
  }
  const curl = curlSnippet(call)

  return (
    <div className='space-y-4'>
      <div
        role='note'
        className='flex items-start gap-2 rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'
      >
        <AlertTriangle className='mt-0.5 h-4 w-4 shrink-0' />
        <p>
          This secret is shown once. Closing this dialog forgets it; rotate the secret to get a new
          one.
        </p>
      </div>
      <Secret label='Client ID' value={revealed.clientId} />
      <Secret label='Client secret' value={revealed.secret} />

      <div className='space-y-1'>
        <div className='flex items-center justify-between'>
          <div className='text-xs font-medium text-muted-foreground'>Get a token with curl</div>
          <CopyButton text={curl} label='Copy curl command' />
        </div>
        <pre className='overflow-x-auto rounded-md border bg-muted p-3 font-mono text-xs'>
          {curl}
        </pre>
      </div>

      <div className='space-y-1'>
        <div className='text-xs font-medium text-muted-foreground'>
          n8n: OAuth2 API credential
        </div>
        <dl className='divide-y rounded-md border text-xs'>
          {n8nFields(call).map((field) => (
            <div key={field.label} className='flex items-start justify-between gap-3 px-3 py-1.5'>
              <dt className='shrink-0 text-muted-foreground'>{field.label}</dt>
              <dd className='min-w-0 break-all text-right font-mono'>{field.value}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  )
}

function Progress({ event }: { event: FlowEvent }) {
  const text =
    event.phase === 'generating'
      ? 'Generating a one-time key in your browser…'
      : event.phase === 'requesting'
        ? 'Asking the data plane…'
        : 'Waiting for the data plane to create the credentials…'

  return (
    <p className='flex items-center gap-2 text-sm text-muted-foreground' role='status'>
      <Spinner />
      {text}
    </p>
  )
}

export function AutonomousDialogBody({
  mode,
  event,
  onCreate,
  onRotate,
  onClose,
}: Props & { mode: AutonomousMode }) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')

  const running = event !== null && event.phase !== 'ready' && event.phase !== 'failed'
  const rotating = mode.kind === 'rotate'

  return (
    <>
        {event === null && !rotating && (
          <form
            className='space-y-4'
            onSubmit={(submitted) => {
              submitted.preventDefault()
              if (name.trim() !== '') onCreate(name.trim(), description.trim())
            }}
          >
            <Field label='Name' htmlFor='autonomous-name'>
              <Input
                id='autonomous-name'
                autoFocus
                value={name}
                maxLength={100}
                placeholder='e.g. Nightly backup report'
                onChange={(changed) => setName(changed.target.value)}
              />
            </Field>
            <Field label='Description' htmlFor='autonomous-description' hint='Optional.'>
              <textarea
                id='autonomous-description'
                className={TEXTAREA_CLASS}
                rows={2}
                maxLength={500}
                value={description}
                onChange={(changed) => setDescription(changed.target.value)}
              />
            </Field>
            <DialogFooter>
              <Button type='button' variant='outline' onClick={onClose}>
                Cancel
              </Button>
              <Button type='submit' disabled={name.trim() === ''}>
                Create agent
              </Button>
            </DialogFooter>
          </form>
        )}

        {event === null && rotating && (
          <DialogFooter>
            <Button variant='outline' onClick={onClose}>
              Cancel
            </Button>
            <Button variant='destructive' onClick={onRotate}>
              Rotate secret
            </Button>
          </DialogFooter>
        )}

        {running && event && <Progress event={event} />}

        {event?.phase === 'failed' && (
          <>
            <p role='alert' className='text-sm text-destructive'>
              {event.message}
            </p>
            <DialogFooter>
              <Button variant='outline' onClick={onClose}>
                Close
              </Button>
            </DialogFooter>
          </>
        )}

        {event?.phase === 'ready' && (
          <>
            <Revealing revealed={event.revealed} />
            <DialogFooter>
              <Button onClick={onClose}>Done</Button>
            </DialogFooter>
          </>
        )}
    </>
  )
}

export function AutonomousAgentDialog(props: Props) {
  const { mode, onClose } = props
  const rotating = mode?.kind === 'rotate'
  const title = rotating
    ? `Rotate the secret of ${displayName(mode.client)}`
    : 'New autonomous agent'

  return (
    <Dialog open={mode !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className='max-h-[90vh] overflow-y-auto sm:max-w-xl'>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {rotating
              ? 'The current secret stops working at once and a new one is created. Anything using the old one must be updated.'
              : 'For a workflow or a bot acting for itself. Its credentials are sealed for this browser tab and shown once.'}
          </DialogDescription>
        </DialogHeader>
        {mode && <AutonomousDialogBody {...props} mode={mode} />}
      </DialogContent>
    </Dialog>
  )
}
