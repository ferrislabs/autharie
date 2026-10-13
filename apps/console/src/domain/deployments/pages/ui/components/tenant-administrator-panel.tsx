import { useState } from 'react'
import { Check, Copy, KeyRound, TriangleAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  ADMINISTRATOR_COPY,
  type RevealedAdministrator,
  type TenantAdministratorView,
} from '../../../tenant-realm'

interface Props {
  administrator: TenantAdministratorView
  revealed?: RevealedAdministrator
  revealing: boolean
  error?: string
  onReveal: () => void
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false)

  return (
    <Button
      type='button'
      variant='outline'
      size='sm'
      onClick={() => {
        void navigator.clipboard?.writeText(text)
        setCopied(true)
        window.setTimeout(() => setCopied(false), 2000)
      }}
    >
      {copied ? <Check className='h-3.5 w-3.5' /> : <Copy className='h-3.5 w-3.5' />}
      {copied ? 'Copied' : label}
    </Button>
  )
}

function Warning({ children }: { children: React.ReactNode }) {
  return (
    <div className='flex gap-3 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'>
      <TriangleAlert className='mt-0.5 h-4 w-4 shrink-0' />
      <p>{children}</p>
    </div>
  )
}

function Field({ label, value, copy }: { label: string; value: string; copy: string }) {
  return (
    <div className='flex items-center justify-between gap-3'>
      <div className='min-w-0'>
        <dt className='text-muted-foreground'>{label}</dt>
        <dd className='break-all font-mono'>{value}</dd>
      </div>
      <CopyButton text={value} label={copy} />
    </div>
  )
}

export function TenantAdministratorPanel({
  administrator,
  revealed,
  revealing,
  error,
  onReveal,
}: Props) {
  if (revealed) {
    return (
      <div className='space-y-3'>
        <Warning>{ADMINISTRATOR_COPY.shownOnce}</Warning>
        <dl className='space-y-2 text-sm'>
          <Field
            label={ADMINISTRATOR_COPY.username}
            value={revealed.username}
            copy={ADMINISTRATOR_COPY.copyUsername}
          />
          <Field
            label={ADMINISTRATOR_COPY.password}
            value={revealed.password}
            copy={ADMINISTRATOR_COPY.copyPassword}
          />
        </dl>
      </div>
    )
  }

  switch (administrator.credential) {
    case 'pending':
      return <p className='text-sm text-muted-foreground'>{ADMINISTRATOR_COPY.pending}</p>
    case 'revealed':
      return <p className='text-sm text-muted-foreground'>{ADMINISTRATOR_COPY.revealed}</p>
    case 'expired':
      return <p className='text-sm text-muted-foreground'>{ADMINISTRATOR_COPY.expired}</p>
    case 'available':
      return (
        <div className='space-y-3'>
          <p className='text-sm text-muted-foreground'>{ADMINISTRATOR_COPY.beforeReveal}</p>
          {administrator.username && (
            <p className='text-sm'>
              <span className='text-muted-foreground'>{ADMINISTRATOR_COPY.username}: </span>
              <span className='font-mono'>{administrator.username}</span>
            </p>
          )}
          <Warning>{ADMINISTRATOR_COPY.shownOnce}</Warning>
          {error && <p className='text-sm text-destructive'>{error}</p>}
          <Button type='button' onClick={onReveal} disabled={revealing}>
            <KeyRound className='h-4 w-4' />
            {revealing ? ADMINISTRATOR_COPY.revealing : ADMINISTRATOR_COPY.reveal}
          </Button>
        </div>
      )
  }
}
