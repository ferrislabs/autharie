import { useState } from 'react'
import { Check, Copy, Eye, EyeOff, TriangleAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Spinner } from '@/components/ui/spinner'
import type { IamView } from '@/domain/identity/iam/iam-view'
import { curlCommands, REPLACE_WARNING, tokenUrlOf, type AdminAccess } from './admin-access'

export interface AdminAccessProps {
  endpoint: string
  view: IamView<AdminAccess>
  onIssue: () => void
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false)

  return (
    <Button
      type='button'
      variant='ghost'
      size='icon'
      aria-label={label}
      onClick={() => {
        void navigator.clipboard.writeText(value)
        setCopied(true)
        window.setTimeout(() => setCopied(false), 1500)
      }}
    >
      {copied ? <Check className='h-4 w-4' /> : <Copy className='h-4 w-4' />}
    </Button>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className='space-y-1'>
      <p className='text-xs font-medium text-muted-foreground'>{label}</p>
      <div className='flex items-center justify-between gap-2 rounded-md border px-3 py-1'>
        {children}
      </div>
    </div>
  )
}

function Secret({ value }: { value: string }) {
  const [shown, setShown] = useState(false)

  return (
    <Field label='Client secret'>
      <code className='min-w-0 flex-1 truncate font-mono text-sm'>
        {shown ? value : '•'.repeat(24)}
      </code>
      <div className='flex shrink-0'>
        <Button
          type='button'
          variant='ghost'
          size='icon'
          aria-label={shown ? 'Hide secret' : 'Reveal secret'}
          onClick={() => setShown(!shown)}
        >
          {shown ? <EyeOff className='h-4 w-4' /> : <Eye className='h-4 w-4' />}
        </Button>
        <CopyButton value={value} label='Copy secret' />
      </div>
    </Field>
  )
}

function Issued({ access, endpoint }: { access: AdminAccess; endpoint: string }) {
  const tokenUrl = tokenUrlOf(endpoint)
  const commands = curlCommands(endpoint)

  return (
    <div className='space-y-4'>
      <p
        role='alert'
        className='flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-sm'
      >
        <TriangleAlert className='h-4 w-4 shrink-0' />
        Shown once. Store it in your secret manager now.
      </p>
      <Field label='Client id'>
        <code className='min-w-0 flex-1 truncate font-mono text-sm'>{access.clientId}</code>
        <CopyButton value={access.clientId} label='Copy client id' />
      </Field>
      <Secret value={access.clientSecret} />
      <Field label='Token URL'>
        <code className='min-w-0 flex-1 truncate font-mono text-sm'>{tokenUrl}</code>
        <CopyButton value={tokenUrl} label='Copy token URL' />
      </Field>
      <div className='space-y-1'>
        <div className='flex items-center justify-between'>
          <p className='text-xs font-medium text-muted-foreground'>
            Create your first admin user
          </p>
          <CopyButton value={commands} label='Copy commands' />
        </div>
        <pre className='overflow-x-auto rounded-md border bg-muted/40 p-3 font-mono text-xs'>
          {commands}
        </pre>
      </div>
    </div>
  )
}

export function AdminAccessCard({ endpoint, view, onIssue }: AdminAccessProps) {
  const [confirming, setConfirming] = useState(false)

  return (
    <Card className='lg:col-span-3'>
      <CardHeader>
        <CardTitle>Admin access</CardTitle>
        <CardDescription>
          A client secret that administers this deployment&apos;s FerrisKey. Use it to create
          your first admin user.
        </CardDescription>
      </CardHeader>
      <CardContent className='space-y-4'>
        {view.phase === 'idle' && (
          <Button size='sm' onClick={() => setConfirming(true)}>
            Generate admin access
          </Button>
        )}
        {view.phase === 'loading' && (
          <p role='status' className='flex items-center gap-2 text-sm text-muted-foreground'>
            <Spinner className='size-4' />
            Asking the deployment… this takes a few seconds.
          </p>
        )}
        {view.phase === 'failed' && (
          <div className='space-y-3'>
            <p
              role='alert'
              className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'
            >
              {view.message}
            </p>
            <Button size='sm' variant='outline' onClick={onIssue}>
              Try again
            </Button>
          </div>
        )}
        {view.phase === 'ready' && <Issued access={view.result} endpoint={endpoint} />}
      </CardContent>

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent className='sm:max-w-md'>
          <DialogHeader>
            <DialogTitle>Generate admin access</DialogTitle>
            <DialogDescription>{REPLACE_WARNING}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type='button' variant='ghost' onClick={() => setConfirming(false)}>
              Cancel
            </Button>
            <Button
              type='button'
              onClick={() => {
                setConfirming(false)
                onIssue()
              }}
            >
              Confirm
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  )
}
