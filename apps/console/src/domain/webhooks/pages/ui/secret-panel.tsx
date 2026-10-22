import { useState } from 'react'
import { Check, Copy, Eye, EyeOff, KeyRound, RotateCw } from 'lucide-react'
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
import type { SecretPanel as SecretState } from '../../secret'

interface Props {
  panel: SecretState
  revealed: string | null
  isRevealing: boolean
  isRotating: boolean
  canManage: boolean
  error?: string
  onReveal: () => void
  onHide: () => void
  onRotate: () => void
}

export function SecretPanel({
  panel,
  revealed,
  isRevealing,
  isRotating,
  canManage,
  error,
  onReveal,
  onHide,
  onRotate,
}: Props) {
  const [confirming, setConfirming] = useState(false)
  const busy = panel.kind === 'generating' || panel.kind === 'rotating'
  const ready = panel.kind === 'ready'

  return (
    <div className='space-y-4 rounded-lg border bg-card p-5'>
      <div className='flex items-start justify-between gap-4'>
        <div className='space-y-1'>
          <h2 className='flex items-center gap-2 text-base font-semibold'>
            <KeyRound className='h-4 w-4 text-muted-foreground' />
            Signing secret
          </h2>
          <p className='text-sm text-muted-foreground'>
            Your service uses it to check the x-ferriskey-signature header of each delivery.
          </p>
        </div>
        {ready && <span className='text-sm text-muted-foreground'>Version {panel.version}</span>}
      </div>

      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      <Status panel={panel} />

      {revealed !== null && <RevealedSecret secret={revealed} />}

      {canManage && (
        <div className='flex flex-wrap items-center gap-2'>
          {ready &&
            (revealed === null ? (
              <Button variant='outline' size='sm' onClick={onReveal} disabled={isRevealing}>
                {isRevealing ? <Spinner className='size-3.5' /> : <Eye className='h-3.5 w-3.5' />}
                Reveal
              </Button>
            ) : (
              <Button variant='outline' size='sm' onClick={onHide}>
                <EyeOff className='h-3.5 w-3.5' />
                Hide
              </Button>
            ))}
          <Button
            variant='outline'
            size='sm'
            onClick={() => setConfirming(true)}
            disabled={busy || isRotating || panel.kind === 'unavailable'}
          >
            <RotateCw className='h-3.5 w-3.5' />
            Rotate
          </Button>
          {ready && (
            <span className='text-xs text-muted-foreground'>
              Shown to you, recorded in the audit trail.
            </span>
          )}
        </div>
      )}

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Rotate the signing secret</DialogTitle>
            <DialogDescription>
              A new secret replaces the current one. Your service must switch to the new secret, or
              it will reject every delivery signed with it. Reveal the new secret once it is ready.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant='outline' onClick={() => setConfirming(false)}>
              Cancel
            </Button>
            <Button
              disabled={isRotating}
              onClick={() => {
                setConfirming(false)
                onRotate()
              }}
            >
              Rotate secret
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function Status({ panel }: { panel: SecretState }) {
  switch (panel.kind) {
    case 'generating':
      return <Waiting>Secret being generated…</Waiting>
    case 'rotating':
      return <Waiting>New secret being generated…</Waiting>
    case 'stalled':
      return (
        <p className='text-sm text-muted-foreground'>
          {panel.rotating ? 'The new secret' : 'The secret'} is taking longer than expected. Reload
          this page in a minute. If it is still missing, check that the deployment is running.
        </p>
      )
    case 'rotation-failed':
      return (
        <p role='alert' className='text-sm text-destructive'>
          The secret could not be rotated: {panel.message} The previous secret still works. Try
          again in a moment.
        </p>
      )
    case 'unavailable':
      return (
        <p className='text-sm text-muted-foreground'>
          This webhook failed to apply, so it has no secret yet. Fix the problem shown in its state,
          and one will be generated.
        </p>
      )
    case 'ready':
      return null
  }
}

function Waiting({ children }: { children: React.ReactNode }) {
  return (
    <p className='flex items-center gap-2 text-sm text-muted-foreground'>
      <Spinner className='size-3.5' />
      {children}
    </p>
  )
}

function RevealedSecret({ secret }: { secret: string }) {
  const [visible, setVisible] = useState(false)
  const [copied, setCopied] = useState(false)

  return (
    <div className='flex items-center gap-2'>
      <Input
        readOnly
        aria-label='Signing secret'
        type={visible ? 'text' : 'password'}
        value={secret}
        className='font-mono'
      />
      <Button
        variant='outline'
        size='icon'
        aria-label={visible ? 'Mask the secret' : 'Show the secret'}
        onClick={() => setVisible(!visible)}
      >
        {visible ? <EyeOff className='h-4 w-4' /> : <Eye className='h-4 w-4' />}
      </Button>
      <Button
        variant='outline'
        size='icon'
        aria-label='Copy the secret'
        onClick={() => {
          void navigator.clipboard?.writeText(secret).then(() => setCopied(true))
        }}
      >
        {copied ? <Check className='h-4 w-4' /> : <Copy className='h-4 w-4' />}
      </Button>
    </div>
  )
}
