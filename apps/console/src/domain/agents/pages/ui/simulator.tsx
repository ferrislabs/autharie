import { useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { StatusBadge } from '@/components/ui/status-badge'
import { simulate, type CedarApi, type NamedPolicy, type Outcome } from '../../cedar-engine'
import { INITIAL_DRAFT, scenarioOf, type SimulationDraft } from '../../simulation-draft'
import { TOOL_CATALOGUE, toolId } from '../../tool-catalogue'
import { Field, TEXTAREA_CLASS } from './field'

interface Props {
  api: CedarApi | null
  isLoading: boolean
  failed: boolean
  policies: NamedPolicy[]
  blocked: boolean
}

function run(
  api: CedarApi,
  policies: NamedPolicy[],
  ran: { draft: SimulationDraft; at: Date },
): Outcome | string {
  try {
    return simulate(api, policies, scenarioOf(ran.draft, ran.at))
  } catch (error) {
    return error instanceof Error ? error.message : 'The simulation failed.'
  }
}

export function Simulator({ api, isLoading, failed, policies, blocked }: Props) {
  const [draft, setDraft] = useState<SimulationDraft>(INITIAL_DRAFT)
  const [ran, setRan] = useState<{ draft: SimulationDraft; at: Date } | null>(null)

  const result = useMemo(
    () => (api && ran && !blocked ? run(api, policies, ran) : null),
    [api, ran, blocked, policies],
  )

  const set = (patch: Partial<SimulationDraft>) => setDraft({ ...draft, ...patch })

  if (failed) {
    return (
      <p className='text-sm text-muted-foreground'>
        The simulator could not be loaded. The policy is still checked by the control plane when you save.
      </p>
    )
  }

  return (
    <div className='space-y-4'>
      <p className='text-sm text-muted-foreground'>
        Try a call against this policy and the other enabled ones. A forbid wins; no permit means deny.
      </p>

      <div className='grid gap-4 sm:grid-cols-2'>
        <Field label='Caller' htmlFor='sim-kind'>
          <Select value={draft.kind} onValueChange={(kind) => set({ kind: kind as SimulationDraft['kind'] })}>
            <SelectTrigger id='sim-kind' className='w-full'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value='user'>A user, through an agent</SelectItem>
              <SelectItem value='agent'>An autonomous agent</SelectItem>
            </SelectContent>
          </Select>
        </Field>

        <Field label={draft.kind === 'user' ? 'Agent client id' : 'Client id'} htmlFor='sim-agent'>
          <Input
            id='sim-agent'
            value={draft.agent}
            spellCheck={false}
            onChange={(event) => set({ agent: event.target.value })}
          />
        </Field>

        {draft.kind === 'user' && (
          <>
            <Field label='User id' htmlFor='sim-user'>
              <Input
                id='sim-user'
                value={draft.userId}
                spellCheck={false}
                onChange={(event) => set({ userId: event.target.value })}
              />
            </Field>
            <Field label='Groups' htmlFor='sim-groups' hint='One per line.'>
              <textarea
                id='sim-groups'
                className={TEXTAREA_CLASS}
                rows={2}
                value={draft.groups}
                onChange={(event) => set({ groups: event.target.value })}
              />
            </Field>
            <Field label='Roles' htmlFor='sim-roles' hint='One per line.'>
              <textarea
                id='sim-roles'
                className={TEXTAREA_CLASS}
                rows={2}
                value={draft.roles}
                onChange={(event) => set({ roles: event.target.value })}
              />
            </Field>
          </>
        )}

        <Field label='Tool' htmlFor='sim-tool'>
          <Select value={draft.tool} onValueChange={(tool) => set({ tool })}>
            <SelectTrigger id='sim-tool' className='w-full'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TOOL_CATALOGUE.map((tool) => (
                <SelectItem key={toolId(tool)} value={toolId(tool)}>
                  {toolId(tool)}
                  {tool.readOnly ? ' (read-only)' : ''}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>

        <Field label='Time (UTC)' htmlFor='sim-time' hint='Leave empty for now.'>
          <Input
            id='sim-time'
            type='datetime-local'
            value={draft.time}
            onChange={(event) => set({ time: event.target.value })}
          />
        </Field>
      </div>

      <div className='flex items-center gap-3'>
        <Button
          type='button'
          size='sm'
          disabled={!api || isLoading || blocked}
          onClick={() => setRan({ draft, at: new Date() })}
        >
          Simulate
        </Button>
        {isLoading && <span className='text-sm text-muted-foreground'>Loading the engine…</span>}
        {blocked && <span className='text-sm text-muted-foreground'>Fix the policy first.</span>}
      </div>

      {typeof result === 'string' && (
        <p role='alert' className='text-sm text-destructive'>
          {result}
        </p>
      )}

      {result !== null && typeof result !== 'string' && (
        <div className='space-y-2 rounded-md border p-3' role='status'>
          <StatusBadge tone={result.decision === 'allow' ? 'success' : 'danger'}>
            {result.decision === 'allow' ? 'Allow' : 'Deny'}
          </StatusBadge>
          <p className='text-sm'>
            {result.determining.length === 0
              ? 'No policy applied, so the call is denied.'
              : `Decided by ${result.determining.map((policy) => policy.name).join(', ')}.`}
          </p>
          {result.errors.map((issue) => (
            <p key={issue.message} className='text-xs text-destructive'>
              {issue.message}
            </p>
          ))}
        </div>
      )}
    </div>
  )
}
