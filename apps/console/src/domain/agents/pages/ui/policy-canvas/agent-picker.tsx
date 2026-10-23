import { useId, useState } from 'react'
import { X } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { agentLabel, resolveAgent, type KnownAgent } from '../../../registry'
import { useKnownAgents } from './known-agents'

function Options({ id, agents }: { id: string; agents: KnownAgent[] }) {
  return (
    <datalist id={id}>
      {agents.map((agent) => (
        <option key={agent.id} value={agent.id}>
          {agent.label}
        </option>
      ))}
    </datalist>
  )
}

export function AgentListInput({
  id,
  values,
  disabled,
  onChange,
}: {
  id: string
  values: string[]
  disabled?: boolean
  onChange: (values: string[]) => void
}) {
  const known = useKnownAgents()
  const listId = useId()
  const [text, setText] = useState('')

  const commit = (raw: string) => {
    const resolved = resolveAgent(raw, known)
    setText('')
    if (resolved !== '' && !values.includes(resolved)) onChange([...values, resolved])
  }

  return (
    <div className='space-y-2'>
      {values.length > 0 && (
        <ul className='flex flex-wrap gap-1.5'>
          {values.map((value) => (
            <li
              key={value}
              className='inline-flex items-center gap-1 rounded-full border bg-muted px-2 py-0.5 text-xs'
              title={value}
            >
              {agentLabel(value, known)}
              {!disabled && (
                <button
                  type='button'
                  aria-label={`Remove ${agentLabel(value, known)}`}
                  className='text-muted-foreground hover:text-foreground'
                  onClick={() => onChange(values.filter((entry) => entry !== value))}
                >
                  <X className='size-3' />
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <Input
        id={id}
        list={listId}
        value={text}
        disabled={disabled}
        placeholder='Pick an agent by name, or type a client id'
        spellCheck={false}
        onChange={(event) => {
          const next = event.target.value
          if (known.some((agent) => agent.id === next)) commit(next)
          else setText(next)
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ',') {
            event.preventDefault()
            commit(text)
          }
        }}
        onBlur={() => text.trim() !== '' && commit(text)}
      />
      <Options id={listId} agents={known} />
    </div>
  )
}

export function AgentValueInput({
  id,
  value,
  disabled,
  placeholder,
  onChange,
}: {
  id: string
  value: string
  disabled?: boolean
  placeholder: string
  onChange: (value: string) => void
}) {
  const known = useKnownAgents()
  const listId = useId()
  const autonomous = known.filter((agent) => agent.autonomous)
  const named = known.find((agent) => agent.id === value)

  return (
    <>
      <Input
        id={id}
        list={listId}
        autoFocus={!disabled}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(event) => onChange(event.target.value)}
        onBlur={() => onChange(resolveAgent(value, autonomous))}
      />
      <Options id={listId} agents={autonomous} />
      {named && <p className='text-xs text-muted-foreground'>{named.label}</p>}
    </>
  )
}
