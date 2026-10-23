import { AlertTriangle, ChevronRight, ExternalLink } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import type { RuleBox } from '../../../canvas-model'
import { toggleItem } from '../../../rule-form'
import type { Effect, When, WhichAgents } from '../../../types'
import { AgentListInput } from './agent-picker'
import { EditorShell, Row, Segmented, TEXTAREA } from './editor-parts'

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
const WEEKDAYS = [1, 2, 3, 4, 5]
const OFFICE_HOURS = { from: 9, to: 18 }

function HourSelect({
  id,
  label,
  value,
  min,
  max,
  disabled,
  onChange,
}: {
  id: string
  label: string
  value: number
  min: number
  max: number
  disabled: boolean
  onChange: (value: number) => void
}) {
  const hours = Array.from({ length: max - min + 1 }, (_, index) => min + index)

  return (
    <div className='flex-1 space-y-1'>
      <label htmlFor={id} className='text-xs text-muted-foreground'>
        {label}
      </label>
      <Select value={String(value)} disabled={disabled} onValueChange={(next) => onChange(Number(next))}>
        <SelectTrigger id={id} className='w-full'>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {hours.map((hour) => (
            <SelectItem key={hour} value={String(hour)}>
              {String(hour).padStart(2, '0')}:00
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

function WhenEditor({
  when,
  disabled,
  onChange,
}: {
  when: When | null
  disabled: boolean
  onChange: (when: When | null) => void
}) {
  return (
    <div className='space-y-3'>
      <div className='flex items-center justify-between gap-3'>
        <label htmlFor='rule-limit' className='text-sm'>
          Only at certain times
        </label>
        <Switch
          id='rule-limit'
          checked={when !== null}
          disabled={disabled}
          onCheckedChange={(on) => onChange(on ? { weekdays: WEEKDAYS, hours: null } : null)}
        />
      </div>
      {when && (
        <>
          <div className='flex flex-wrap gap-1' role='group' aria-label='Days of the week'>
            {DAYS.map((day, index) => {
              const value = index + 1
              const active = when.weekdays.includes(value)
              return (
                <button
                  key={day}
                  type='button'
                  aria-pressed={active}
                  disabled={disabled}
                  className={cn(
                    'rounded-md border px-2 py-1 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed',
                    active ? 'border-primary bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-accent',
                  )}
                  onClick={() =>
                    onChange({ ...when, weekdays: toggleItem(when.weekdays, value).sort((a, b) => a - b) })
                  }
                >
                  {day}
                </button>
              )
            })}
          </div>
          <div className='flex items-center justify-between gap-3'>
            <label htmlFor='rule-hours' className='text-sm'>
              Between certain hours (UTC)
            </label>
            <Switch
              id='rule-hours'
              checked={when.hours !== null}
              disabled={disabled}
              onCheckedChange={(on) => onChange({ ...when, hours: on ? OFFICE_HOURS : null })}
            />
          </div>
          {when.hours && (
            <div className='flex gap-2'>
              <HourSelect
                id='rule-from'
                label='From'
                value={when.hours.from}
                min={0}
                max={23}
                disabled={disabled}
                onChange={(from) => onChange({ ...when, hours: { from, to: when.hours?.to ?? 18 } })}
              />
              <HourSelect
                id='rule-to'
                label='To'
                value={when.hours.to}
                min={1}
                max={24}
                disabled={disabled}
                onChange={(to) => onChange({ ...when, hours: { from: when.hours?.from ?? 9, to } })}
              />
            </div>
          )}
        </>
      )}
    </div>
  )
}

export function RuleEditor({
  box,
  issues,
  cedar,
  readOnly,
  canOpenAdvanced,
  onChange,
  onOpenAdvanced,
  onClose,
}: {
  box: RuleBox
  issues: string[]
  cedar: string | null
  readOnly: boolean
  canOpenAdvanced: boolean
  onChange: (box: RuleBox) => void
  onOpenAdvanced: () => void
  onClose: () => void
}) {
  const agents = box.agents

  function setAgents(next: WhichAgents) {
    onChange({ ...box, agents: next })
  }

  return (
    <EditorShell
      title='Rule'
      subtitle={box.policyId === null ? 'Not published yet' : 'Published rule'}
      onClose={onClose}
    >
      <Row label='Name' htmlFor='rule-name'>
        <Input
          id='rule-name'
          value={box.name}
          disabled={readOnly}
          placeholder='What does this rule do?'
          onChange={(event) => onChange({ ...box, name: event.target.value })}
        />
      </Row>

      <Row label='Effect'>
        <Segmented<Effect>
          label='Effect'
          value={box.effect}
          disabled={readOnly}
          options={[
            { value: 'permit', label: 'Allow', tone: 'permit' },
            { value: 'forbid', label: 'Deny', tone: 'forbid' },
          ]}
          onChange={(effect) => onChange({ ...box, effect })}
        />
        <p className='text-xs text-muted-foreground'>A deny always wins over an allow.</p>
      </Row>

      <Row label='Through which agent'>
        <Segmented<WhichAgents['kind']>
          label='Through which agent'
          value={agents.kind}
          disabled={readOnly}
          options={[
            { value: 'any', label: 'Any agent' },
            { value: 'clients', label: 'Specific agents' },
          ]}
          onChange={(kind) => setAgents(kind === 'any' ? { kind: 'any' } : { kind: 'clients', clientIds: [] })}
        />
        {agents.kind === 'clients' && (
          <AgentListInput
            id='rule-clients'
            values={agents.clientIds}
            disabled={readOnly}
            onChange={(clientIds) => setAgents({ kind: 'clients', clientIds })}
          />
        )}
      </Row>

      <Row label='When'>
        <WhenEditor when={box.when} disabled={readOnly} onChange={(when) => onChange({ ...box, when })} />
      </Row>

      <div className='flex items-center justify-between gap-3'>
        <label htmlFor='rule-enabled' className='text-sm'>
          Enabled
        </label>
        <Switch
          id='rule-enabled'
          checked={box.enabled}
          disabled={readOnly}
          onCheckedChange={(enabled) => onChange({ ...box, enabled })}
        />
      </div>

      <Row label='Description' htmlFor='rule-description'>
        <textarea
          id='rule-description'
          className={TEXTAREA}
          rows={2}
          value={box.description}
          disabled={readOnly}
          onChange={(event) => onChange({ ...box, description: event.target.value })}
        />
      </Row>

      {issues.length > 0 && (
        <ul
          role='alert'
          className='space-y-1 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200'
        >
          {issues.map((issue) => (
            <li key={issue} className='flex gap-1.5'>
              <AlertTriangle className='mt-0.5 size-3 shrink-0' />
              {issue}
            </li>
          ))}
        </ul>
      )}

      <Collapsible className='rounded-md border'>
        <CollapsibleTrigger className='group flex w-full items-center gap-1.5 px-3 py-2 text-xs font-medium'>
          <ChevronRight className='size-3.5 transition-transform group-data-[state=open]:rotate-90' />
          Cedar
        </CollapsibleTrigger>
        <CollapsibleContent>
          <pre className='max-h-40 overflow-auto border-t bg-muted/40 px-3 py-2 font-mono text-[11px] leading-relaxed'>
            {cedar ?? 'Fix the issues above to see the generated Cedar.'}
          </pre>
        </CollapsibleContent>
      </Collapsible>

      {canOpenAdvanced && (
        <Button type='button' variant='outline' size='sm' className='w-full' onClick={onOpenAdvanced}>
          <ExternalLink className='size-4' />
          Open in advanced editor
        </Button>
      )}
    </EditorShell>
  )
}
