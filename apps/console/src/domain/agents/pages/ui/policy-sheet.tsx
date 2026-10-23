import { useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Spinner } from '@/components/ui/spinner'
import { isValid, validatePolicy, type NamedPolicy, type Validation } from '../../cedar-engine'
import { useCedar } from '../../hooks/use-cedar'
import {
  cedarOf,
  draftFromPolicy,
  draftFromTemplate,
  nameIssue,
  switchToCedar,
  toInput,
  type PolicyDraft,
} from '../../policy-draft'
import type { PolicyTemplateDraft } from '../../policy-templates'
import type { AgentPolicy, AgentPolicyInput } from '../../types'
import { CedarEditor } from './cedar-editor'
import { Field, TEXTAREA_CLASS } from './field'
import { RuleBuilder } from './rule-builder'
import { Simulator } from './simulator'

interface Props {
  open: boolean
  policy: AgentPolicy | null
  initial?: PolicyTemplateDraft
  others: AgentPolicy[]
  isSaving: boolean
  error?: string
  onSave: (input: AgentPolicyInput) => void
  onClose: () => void
}

const DRAFT_ID = 'draft'

export function ValidationPanel({
  validation,
  checking,
  failed,
  issues,
}: {
  validation: Validation | null
  checking: boolean
  failed: boolean
  issues: string[]
}) {
  if (issues.length > 0) {
    return (
      <ul className='space-y-1 text-sm text-destructive' role='alert'>
        {issues.map((issue) => (
          <li key={issue}>{issue}</li>
        ))}
      </ul>
    )
  }

  if (failed) {
    return (
      <p className='text-sm text-muted-foreground'>
        The browser check could not be loaded. The control plane still checks the policy when you save.
      </p>
    )
  }

  if (checking || validation === null) {
    return <p className='text-sm text-muted-foreground'>Checking against the schema…</p>
  }

  const problems = [...validation.syntax, ...validation.errors]

  if (problems.length > 0) {
    return (
      <ul className='space-y-2 text-sm text-destructive' role='alert'>
        {problems.map((problem) => (
          <li key={problem.message}>
            {problem.message}
            {problem.help && <span className='block text-xs text-muted-foreground'>{problem.help}</span>}
          </li>
        ))}
      </ul>
    )
  }

  return (
    <div className='space-y-1 text-sm'>
      <p className='text-green-700 dark:text-green-300'>Valid against the FerrisKey schema.</p>
      {validation.warnings.map((warning) => (
        <p key={warning.message} className='text-xs text-amber-700 dark:text-amber-300'>
          {warning.message}
        </p>
      ))}
    </div>
  )
}

function PolicyForm({
  policy,
  initial,
  others,
  isSaving,
  error,
  onSave,
  onClose,
}: Omit<Props, 'open'>) {
  const [draft, setDraft] = useState<PolicyDraft>(() =>
    policy === null && initial ? draftFromTemplate(initial) : draftFromPolicy(policy),
  )
  const [touched, setTouched] = useState(false)
  const cedar = useCedar(true)

  const { cedar: text, issues } = useMemo(() => cedarOf(draft), [draft])
  const nameProblem = touched ? nameIssue(draft.name) : null

  const validation = useMemo(
    () => (cedar.api && text !== '' ? validatePolicy(cedar.api, text) : null),
    [cedar.api, text],
  )

  const simulated = useMemo<NamedPolicy[]>(() => {
    const rest = others
      .filter((other) => other.enabled && other.id !== policy?.id)
      .map((other) => ({ id: `policy-${other.id}`, name: other.name, cedar: other.cedar }))
    return text === ''
      ? rest
      : [{ id: DRAFT_ID, name: draft.name.trim() || 'This policy', cedar: text }, ...rest]
  }, [others, policy?.id, text, draft.name])

  const input = toInput(draft)
  const invalid = validation !== null && !isValid(validation)
  const canSave = input !== null && !invalid && !isSaving
  const blocked = text === '' || invalid

  return (
    <>
      <SheetHeader>
        <SheetTitle>{policy ? 'Edit policy' : 'New policy'}</SheetTitle>
        <SheetDescription>
          A policy says which agent may call which tool, for whom and when. It is checked here and again before it reaches the deployment.
        </SheetDescription>
      </SheetHeader>

      <div className='flex-1 space-y-6 overflow-y-auto px-4 pb-4'>
        <Field label='Name' htmlFor='policy-name' hint={nameProblem ?? undefined}>
          <Input
            id='policy-name'
            value={draft.name}
            maxLength={120}
            placeholder='Support reads on weekdays'
            aria-invalid={nameProblem !== null}
            onBlur={() => setTouched(true)}
            onChange={(event) => setDraft({ ...draft, name: event.target.value })}
          />
        </Field>

        <Field label='Description' htmlFor='policy-description'>
          <textarea
            id='policy-description'
            className={TEXTAREA_CLASS}
            rows={2}
            value={draft.description}
            onChange={(event) => setDraft({ ...draft, description: event.target.value })}
          />
        </Field>

        <div className='flex items-center justify-between rounded-lg border p-4'>
          <div>
            <p className='text-sm font-medium'>Enabled</p>
            <p className='text-xs text-muted-foreground'>
              A disabled policy is kept here and not sent to the deployment.
            </p>
          </div>
          <Switch
            checked={draft.enabled}
            aria-label='Enabled'
            onCheckedChange={(enabled) => setDraft({ ...draft, enabled })}
          />
        </div>

        {draft.mode === 'builder' ? (
          <>
            <RuleBuilder form={draft.form} onChange={(form) => setDraft({ ...draft, form })} />

            <section className='space-y-2'>
              <div className='flex items-center justify-between'>
                <h3 className='text-sm font-semibold'>Cedar</h3>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  disabled={issues.length > 0}
                  onClick={() => setDraft(switchToCedar(draft))}
                >
                  Edit as Cedar
                </Button>
              </div>
              <CedarEditor id='policy-cedar-preview' value={text} readOnly rows={8} />
              <p className='text-xs text-muted-foreground'>
                What the rule above compiles to. Editing it as Cedar is one way: the rule cannot be rebuilt from hand-written text.
              </p>
            </section>
          </>
        ) : (
          <section className='space-y-2'>
            <h3 className='text-sm font-semibold'>Cedar</h3>
            <CedarEditor
              id='policy-cedar'
              value={draft.cedar}
              onChange={(value) => setDraft({ ...draft, cedar: value })}
            />
            <p className='text-xs text-muted-foreground'>
              Entities are FerrisKey::User, Group, Agent, Tool and McpServer; the action is
              FerrisKey::Action::&quot;call_tool&quot;; the context holds agent, delegated, arguments and time.
            </p>
          </section>
        )}

        <section className='space-y-2'>
          <h3 className='text-sm font-semibold'>Check</h3>
          <ValidationPanel
            validation={validation}
            checking={cedar.isLoading}
            failed={cedar.failed}
            issues={issues}
          />
        </section>

        <section className='space-y-2'>
          <h3 className='text-sm font-semibold'>Simulate</h3>
          <Simulator
            api={cedar.api}
            isLoading={cedar.isLoading}
            failed={cedar.failed}
            policies={simulated}
            blocked={blocked}
          />
        </section>

        {error && (
          <p role='alert' className='text-sm text-destructive'>
            {error}
          </p>
        )}
      </div>

      <SheetFooter className='flex-row justify-end border-t'>
        <Button type='button' variant='outline' onClick={onClose}>
          Cancel
        </Button>
        <Button
          type='button'
          disabled={!canSave}
          onClick={() => {
            setTouched(true)
            if (input) onSave(input)
          }}
        >
          {isSaving && <Spinner />}
          {policy ? 'Save policy' : 'Create policy'}
        </Button>
      </SheetFooter>
    </>
  )
}

export function PolicySheet({ open, policy, ...rest }: Props) {
  return (
    <Sheet open={open} onOpenChange={(next) => !next && rest.onClose()}>
      <SheetContent className='w-full gap-0 sm:max-w-2xl'>
        {open && <PolicyForm key={policy?.id ?? 'new'} policy={policy} {...rest} />}
      </SheetContent>
    </Sheet>
  )
}
