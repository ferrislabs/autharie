import { useState } from 'react'
import { Mail, MailPlus } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState, Section, SectionPage } from '@/components/layout/page'
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { StatusBadge } from '@/components/ui/status-badge'
import {
  describeEmailProblem,
  describeHostProblem,
  describePasswordProblem,
  describePortProblem,
  describeUsernameProblem,
  ENCRYPTION_LABELS,
  ENCRYPTIONS,
  isDirty,
  isValid,
  portHint,
  toForm,
  type SmtpForm,
} from '../../smtp-form'
import { smtpBadge } from '../../smtp-state'

export const TEST_NOTE =
  'FerrisKey cannot send a test email. Applied means FerrisKey accepted these settings, not that the server delivered anything.'

const PASSWORD_SET = 'A password is set — leave empty to keep it'

interface Props {
  isLoading: boolean
  unsupported: boolean
  error?: string
  realm: string
  smtp: Schemas.SmtpView | null
  canManage: boolean
  isSaving: boolean
  isRemoving: boolean
  saveError?: string
  removeError?: string
  onSave: (form: SmtpForm) => void
  onRemove: () => void
}

export function PageEmailProvider({
  isLoading,
  unsupported,
  error,
  realm,
  smtp,
  canManage,
  isSaving,
  isRemoving,
  saveError,
  removeError,
  onSave,
  onRemove,
}: Props) {
  const description = 'The SMTP server FerrisKey sends this realm’s emails through.'

  if (unsupported) {
    return (
      <SectionPage title='Email provider' description={description}>
        <EmptyState
          icon={<Mail className='h-5 w-5' />}
          title='Email settings are available on FerrisKey deployments'
          description='This deployment does not run FerrisKey, so there is no realm to set an email provider on.'
        />
      </SectionPage>
    )
  }

  return (
    <SectionPage title='Email provider' description={description}>
      {error && <Alert>{error}</Alert>}

      {isLoading ? (
        <Skeleton className='h-64 w-full' />
      ) : (
        <ProviderBody
          key={realm}
          smtp={smtp}
          canManage={canManage}
          isSaving={isSaving}
          isRemoving={isRemoving}
          saveError={saveError}
          removeError={removeError}
          onSave={onSave}
          onRemove={onRemove}
        />
      )}
    </SectionPage>
  )
}

function ProviderBody({
  smtp,
  canManage,
  isSaving,
  isRemoving,
  saveError,
  removeError,
  onSave,
  onRemove,
}: Pick<
  Props,
  | 'smtp'
  | 'canManage'
  | 'isSaving'
  | 'isRemoving'
  | 'saveError'
  | 'removeError'
  | 'onSave'
  | 'onRemove'
>) {
  const [setUp, setSetUp] = useState(false)

  if (!smtp && !setUp) {
    return (
      <EmptyState
        icon={<MailPlus className='h-5 w-5' />}
        title='No email provider'
        description='Without an email provider FerrisKey sends no email from this realm: no verification, no password reset, no invitation. Set up the SMTP server it should use.'
        action={
          canManage ? (
            <Button onClick={() => setSetUp(true)}>
              <MailPlus className='h-3.5 w-3.5' />
              Set up an email provider
            </Button>
          ) : (
            <p className='text-sm text-muted-foreground'>
              Ask someone who can manage instances to set one up.
            </p>
          )
        }
      />
    )
  }

  return (
    <ProviderEditor
      key={smtp?.updated_at ?? 'new'}
      stored={smtp}
      canManage={canManage}
      isSaving={isSaving}
      isRemoving={isRemoving}
      saveError={saveError}
      removeError={removeError}
      onSave={onSave}
      onRemove={() => {
        setSetUp(false)
        onRemove()
      }}
    />
  )
}

function ProviderEditor({
  stored,
  canManage,
  isSaving,
  isRemoving,
  saveError,
  removeError,
  onSave,
  onRemove,
}: {
  stored: Schemas.SmtpView | null
  canManage: boolean
  isSaving: boolean
  isRemoving: boolean
  saveError?: string
  removeError?: string
  onSave: (form: SmtpForm) => void
  onRemove: () => void
}) {
  const [form, setForm] = useState<SmtpForm>(() => toForm(stored))
  const [removing, setRemoving] = useState(false)

  const hasPassword = stored?.password_set ?? false
  const canSave = canManage && !isSaving && isDirty(form, stored) && isValid(form, hasPassword)
  const readOnly = !canManage
  const badge = stored ? smtpBadge(stored.state) : null

  const host = form.host === '' ? null : describeHostProblem(form.host)
  const port = form.port === '' ? null : describePortProblem(form.port)
  const username = form.username === '' ? null : describeUsernameProblem(form.username)
  const password =
    form.password === '' ? null : describePasswordProblem(form.password, hasPassword)
  const fromEmail = form.fromEmail === '' ? null : describeEmailProblem(form.fromEmail)

  return (
    <>
      <Section
        title='SMTP server'
        aside={badge && <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>}
      >
        {saveError && <Alert>{saveError}</Alert>}
        {removeError && <Alert>{removeError}</Alert>}
        {badge?.message && (
          <p className='rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            {badge.message}
          </p>
        )}

        <form
          className='max-w-3xl space-y-5 rounded-lg border p-5'
          noValidate
          onSubmit={(event) => {
            event.preventDefault()
            if (canSave) onSave(form)
          }}
        >
          <div className='grid gap-4 sm:grid-cols-[1fr_8rem]'>
            <Field label='Host' htmlFor='smtp-host' problem={host}>
              <Input
                id='smtp-host'
                value={form.host}
                onChange={(event) => setForm({ ...form, host: event.target.value })}
                placeholder='smtp.example.com'
                readOnly={readOnly}
                aria-invalid={!!host}
                className='font-mono'
              />
            </Field>
            <Field label='Port' htmlFor='smtp-port' problem={port}>
              <Input
                id='smtp-port'
                inputMode='numeric'
                value={form.port}
                onChange={(event) => setForm({ ...form, port: event.target.value })}
                readOnly={readOnly}
                aria-invalid={!!port}
                className='font-mono'
              />
            </Field>
          </div>

          <Field label='Encryption' htmlFor='smtp-encryption' hint={portHint(form.encryption)}>
            <Select
              value={form.encryption}
              onValueChange={(encryption) =>
                setForm({ ...form, encryption: encryption as Schemas.SmtpEncryption })
              }
              disabled={readOnly}
            >
              <SelectTrigger id='smtp-encryption' className='w-48'>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ENCRYPTIONS.map((encryption) => (
                  <SelectItem key={encryption} value={encryption}>
                    {ENCRYPTION_LABELS[encryption]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <div className='grid gap-4 sm:grid-cols-2'>
            <Field label='Username' htmlFor='smtp-username' problem={username}>
              <Input
                id='smtp-username'
                value={form.username}
                onChange={(event) => setForm({ ...form, username: event.target.value })}
                readOnly={readOnly}
                autoComplete='off'
                aria-invalid={!!username}
              />
            </Field>
            <Field
              label='Password'
              htmlFor='smtp-password'
              problem={password}
              hint={hasPassword ? PASSWORD_SET : undefined}
            >
              <Input
                id='smtp-password'
                type='password'
                value={form.password}
                onChange={(event) => setForm({ ...form, password: event.target.value })}
                readOnly={readOnly}
                autoComplete='new-password'
                aria-invalid={!!password}
              />
            </Field>
          </div>

          <div className='grid gap-4 sm:grid-cols-2'>
            <Field label='From name' htmlFor='smtp-from-name' hint='Optional. Shown as the sender.'>
              <Input
                id='smtp-from-name'
                value={form.fromName}
                onChange={(event) => setForm({ ...form, fromName: event.target.value })}
                placeholder='Acme'
                readOnly={readOnly}
              />
            </Field>
            <Field label='From email' htmlFor='smtp-from-email' problem={fromEmail}>
              <Input
                id='smtp-from-email'
                type='email'
                value={form.fromEmail}
                onChange={(event) => setForm({ ...form, fromEmail: event.target.value })}
                placeholder='no-reply@example.com'
                readOnly={readOnly}
                aria-invalid={!!fromEmail}
              />
            </Field>
          </div>

          <p className='text-xs text-muted-foreground'>{TEST_NOTE}</p>

          {canManage && (
            <div className='flex items-center justify-between gap-2 border-t pt-4'>
              <Button type='submit' disabled={!canSave}>
                {isSaving && <Spinner className='size-3.5' />}
                {isSaving ? 'Saving' : 'Save'}
              </Button>
              {stored && (
                <Button
                  type='button'
                  variant='outline'
                  disabled={isSaving || isRemoving}
                  onClick={() => setRemoving(true)}
                >
                  Remove provider
                </Button>
              )}
            </div>
          )}
        </form>
      </Section>

      <RemoveDialog
        open={removing}
        onOpenChange={setRemoving}
        isRemoving={isRemoving}
        onConfirm={() => {
          onRemove()
          setRemoving(false)
        }}
      />
    </>
  )
}

function Alert({ children }: { children: React.ReactNode }) {
  return (
    <p
      role='alert'
      className='rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive'
    >
      {children}
    </p>
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
  htmlFor: string
  problem?: string | null
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

function RemoveDialog({
  open,
  onOpenChange,
  isRemoving,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  isRemoving: boolean
  onConfirm: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Remove email provider</DialogTitle>
          <DialogDescription>
            This realm stops sending email until a provider is set again. The stored password is
            deleted.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant='outline' onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant='destructive' onClick={onConfirm} disabled={isRemoving}>
            Remove provider
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
