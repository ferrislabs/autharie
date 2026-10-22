import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'
import { toggleItem, type RuleForm } from '../../rule-form'
import { SERVERS, TOOL_CATALOGUE, toolId, toolsOf } from '../../tool-catalogue'
import { Field, TEXTAREA_CLASS } from './field'

interface Props {
  form: RuleForm
  onChange: (form: RuleForm) => void
}

const WHO_OPTIONS: { value: RuleForm['who']; label: string }[] = [
  { value: 'any-user', label: 'Any user' },
  { value: 'groups', label: 'Users in groups' },
  { value: 'roles', label: 'Users with roles' },
  { value: 'users', label: 'Specific users' },
  { value: 'agents', label: 'Autonomous agents' },
]

const WHAT_OPTIONS: { value: RuleForm['what']; label: string }[] = [
  { value: 'read-only', label: 'All read-only tools' },
  { value: 'server', label: 'All tools of a server' },
  { value: 'tools', label: 'Specific tools' },
]

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
const ANY_SERVER = 'any'

function ListField({
  id,
  label,
  hint,
  placeholder,
  value,
  onChange,
}: {
  id: string
  label: string
  hint: string
  placeholder: string
  value: string
  onChange: (value: string) => void
}) {
  return (
    <Field label={label} htmlFor={id} hint={hint}>
      <textarea
        id={id}
        className={cn(TEXTAREA_CLASS, 'font-mono')}
        rows={3}
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  )
}

function Block({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className='space-y-3 rounded-lg border p-4'>
      <h3 className='text-sm font-semibold'>{title}</h3>
      {children}
    </section>
  )
}

function HourSelect({
  id,
  label,
  value,
  min,
  max,
  onChange,
}: {
  id: string
  label: string
  value: number
  min: number
  max: number
  onChange: (value: number) => void
}) {
  const hours = Array.from({ length: max - min + 1 }, (_, index) => min + index)

  return (
    <Field label={label} htmlFor={id}>
      <Select value={String(value)} onValueChange={(next) => onChange(Number(next))}>
        <SelectTrigger id={id} className='w-28'>
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
    </Field>
  )
}

export function RuleBuilder({ form, onChange }: Props) {
  const set = (patch: Partial<RuleForm>) => onChange({ ...form, ...patch })

  return (
    <div className='space-y-4'>
      <Block title='Effect'>
        <div className='flex gap-2' role='radiogroup' aria-label='Effect'>
          <Button
            type='button'
            role='radio'
            aria-checked={form.effect === 'permit'}
            variant={form.effect === 'permit' ? 'default' : 'outline'}
            size='sm'
            onClick={() => set({ effect: 'permit' })}
          >
            Permit
          </Button>
          <Button
            type='button'
            role='radio'
            aria-checked={form.effect === 'forbid'}
            variant={form.effect === 'forbid' ? 'destructive' : 'outline'}
            size='sm'
            onClick={() => set({ effect: 'forbid' })}
          >
            Forbid
          </Button>
        </div>
        <p className='text-xs text-muted-foreground'>
          A forbid always wins over a permit. Without a permit, a call is denied.
        </p>
      </Block>

      <Block title='Who'>
        <Field label='Calls made for' htmlFor='rule-who'>
          <Select value={form.who} onValueChange={(who) => set({ who: who as RuleForm['who'] })}>
            <SelectTrigger id='rule-who' className='w-full'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {WHO_OPTIONS.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        {form.who === 'groups' && (
          <ListField
            id='rule-groups'
            label='Groups'
            hint='One group per line. A user in any of them matches.'
            placeholder='support'
            value={form.groups}
            onChange={(groups) => set({ groups })}
          />
        )}
        {form.who === 'roles' && (
          <ListField
            id='rule-roles'
            label='Roles'
            hint='One role per line. A user holding any of them matches.'
            placeholder='viewer'
            value={form.roles}
            onChange={(roles) => set({ roles })}
          />
        )}
        {form.who === 'users' && (
          <ListField
            id='rule-users'
            label='User ids'
            hint='One user id per line.'
            placeholder='7b0f6a52-…'
            value={form.users}
            onChange={(users) => set({ users })}
          />
        )}
        {form.who === 'agents' && (
          <ListField
            id='rule-autonomous'
            label='Client ids'
            hint='One client id per line. These agents act for themselves, with no user.'
            placeholder='nightly-report'
            value={form.autonomousIds}
            onChange={(autonomousIds) => set({ autonomousIds })}
          />
        )}
      </Block>

      <Block title='Which agents'>
        <Field label='Through' htmlFor='rule-agents'>
          <Select
            value={form.agents}
            onValueChange={(agents) => set({ agents: agents as RuleForm['agents'] })}
          >
            <SelectTrigger id='rule-agents' className='w-full'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value='any'>Any agent</SelectItem>
              <SelectItem value='clients'>Specific agents</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        {form.agents === 'clients' && (
          <ListField
            id='rule-client-ids'
            label='Agent client ids'
            hint='One OAuth client id per line.'
            placeholder='claude-desktop'
            value={form.clientIds}
            onChange={(clientIds) => set({ clientIds })}
          />
        )}
      </Block>

      <Block title='What'>
        <Field label='Tools' htmlFor='rule-what'>
          <Select value={form.what} onValueChange={(what) => set({ what: what as RuleForm['what'] })}>
            <SelectTrigger id='rule-what' className='w-full'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {WHAT_OPTIONS.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>

        {form.what === 'read-only' && (
          <Field label='On' htmlFor='rule-read-only-server'>
            <Select
              value={form.readOnlyServer === '' ? ANY_SERVER : form.readOnlyServer}
              onValueChange={(server) => set({ readOnlyServer: server === ANY_SERVER ? '' : server })}
            >
              <SelectTrigger id='rule-read-only-server' className='w-full'>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ANY_SERVER}>Any server</SelectItem>
                {SERVERS.map((server) => (
                  <SelectItem key={server.id} value={server.id}>
                    {server.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        )}

        {form.what === 'server' && (
          <Field label='Server' htmlFor='rule-server'>
            <Select value={form.server} onValueChange={(server) => set({ server })}>
              <SelectTrigger id='rule-server' className='w-full'>
                <SelectValue placeholder='Pick a server' />
              </SelectTrigger>
              <SelectContent>
                {SERVERS.map((server) => (
                  <SelectItem key={server.id} value={server.id}>
                    {server.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        )}

        {form.what === 'tools' && (
          <div className='space-y-3'>
            {SERVERS.map((server) => (
              <fieldset key={server.id} className='space-y-1'>
                <legend className='mb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground'>
                  {server.label}
                </legend>
                {toolsOf(server.id).map((tool) => {
                  const id = toolId(tool)

                  return (
                    <label key={id} className='flex items-center gap-2 text-sm'>
                      <input
                        type='checkbox'
                        checked={form.tools.includes(id)}
                        onChange={() => set({ tools: toggleItem(form.tools, id) })}
                      />
                      <span className='font-mono text-xs'>{tool.name}</span>
                      {tool.readOnly && (
                        <span className='rounded border px-1.5 py-0.5 text-[10px] text-muted-foreground'>
                          read-only
                        </span>
                      )}
                      {tool.destructive && (
                        <span className='rounded border border-red-200 px-1.5 py-0.5 text-[10px] text-red-700 dark:border-red-900 dark:text-red-300'>
                          destructive
                        </span>
                      )}
                    </label>
                  )
                })}
              </fieldset>
            ))}
            <p className='text-xs text-muted-foreground'>
              {form.tools.length} of {TOOL_CATALOGUE.length} tools selected.
            </p>
          </div>
        )}
      </Block>

      <Block title='When'>
        <label className='flex items-center gap-2 text-sm'>
          <input
            type='checkbox'
            checked={form.limitDays}
            onChange={(event) => set({ limitDays: event.target.checked })}
          />
          Only on some weekdays
        </label>
        {form.limitDays && (
          <div className='flex flex-wrap gap-2'>
            {DAYS.map((day, index) => {
              const number = index + 1
              const on = form.weekdays.includes(number)

              return (
                <Button
                  key={day}
                  type='button'
                  size='sm'
                  variant={on ? 'default' : 'outline'}
                  aria-pressed={on}
                  onClick={() => set({ weekdays: toggleItem(form.weekdays, number) })}
                >
                  {day}
                </Button>
              )
            })}
          </div>
        )}

        <label className='flex items-center gap-2 text-sm'>
          <input
            type='checkbox'
            checked={form.limitHours}
            onChange={(event) => set({ limitHours: event.target.checked })}
          />
          Only during some hours (UTC)
        </label>
        {form.limitHours && (
          <div className='flex items-end gap-3'>
            <HourSelect
              id='rule-from'
              label='From'
              value={form.from}
              min={0}
              max={23}
              onChange={(from) => set({ from })}
            />
            <HourSelect
              id='rule-to'
              label='Until'
              value={form.to}
              min={1}
              max={24}
              onChange={(to) => set({ to })}
            />
          </div>
        )}
        <p className='text-xs text-muted-foreground'>
          Times are UTC. A window like 22:00 to 06:00 runs across midnight.
        </p>
      </Block>
    </div>
  )
}
