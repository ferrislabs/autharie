import { useEffect, useRef, useState } from 'react'
import { Search } from 'lucide-react'
import { Input } from '@/components/ui/input'
import {
  groupEvents,
  groupSelection,
  toggleEvent,
  toggleGroup,
  type EventGroup,
  type EventInfo,
} from '../../events'

interface Props {
  catalogue: EventInfo[]
  selected: string[]
  onChange: (selected: string[]) => void
  invalid?: boolean
}

export function EventPicker({ catalogue, selected, onChange, invalid }: Props) {
  const [search, setSearch] = useState('')
  const groups = groupEvents(catalogue, search)

  return (
    <div className={`rounded-lg border ${invalid ? 'border-destructive' : ''}`}>
      <div className='flex items-center gap-2 border-b px-3 py-2'>
        <Search className='h-3.5 w-3.5 text-muted-foreground' />
        <Input
          aria-label='Search events'
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder='Search events, such as user or login'
          className='h-8 border-0 bg-transparent shadow-none focus-visible:ring-0'
        />
        <span className='shrink-0 text-xs text-muted-foreground'>{selected.length} selected</span>
      </div>

      <div className='max-h-80 divide-y overflow-y-auto'>
        {groups.length === 0 && (
          <p className='px-3 py-6 text-center text-sm text-muted-foreground'>
            No event matches “{search.trim()}”.
          </p>
        )}
        {groups.map((group) => (
          <GroupBlock key={group.group} group={group} selected={selected} onChange={onChange} />
        ))}
      </div>
    </div>
  )
}

function GroupBlock({
  group,
  selected,
  onChange,
}: {
  group: EventGroup
  selected: string[]
  onChange: (selected: string[]) => void
}) {
  const selection = groupSelection(group, selected)
  const box = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (box.current) box.current.indeterminate = selection === 'some'
  }, [selection])

  return (
    <div className='px-3 py-2.5'>
      <label className='flex cursor-pointer items-center gap-2 text-sm font-medium'>
        <input
          ref={box}
          type='checkbox'
          className='size-4 accent-primary'
          checked={selection === 'all'}
          onChange={() => onChange(toggleGroup(group, selected))}
          aria-label={`Select all ${group.group} events`}
        />
        <span className='capitalize'>{group.group}</span>
        <span className='text-xs font-normal text-muted-foreground'>
          {group.events.length} {group.events.length === 1 ? 'event' : 'events'}
        </span>
      </label>
      <div className='mt-2 grid gap-1.5 pl-6 sm:grid-cols-2'>
        {group.events.map((name) => (
          <label key={name} className='flex cursor-pointer items-center gap-2 text-sm'>
            <input
              type='checkbox'
              className='size-4 accent-primary'
              checked={selected.includes(name)}
              onChange={() => onChange(toggleEvent(selected, name))}
            />
            <span className='font-mono text-xs'>{name}</span>
          </label>
        ))}
      </div>
    </div>
  )
}
