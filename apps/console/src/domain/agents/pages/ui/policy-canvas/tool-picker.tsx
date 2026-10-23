import { useMemo, useState } from 'react'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { toggleItem } from '../../../rule-form'
import { findTool, nameOfTool, serverOfTool, toolId, type Catalogue } from '../../../tool-catalogue'
import { useCatalogue } from '../catalogue-context'

interface Row {
  id: string
  name: string
  readOnly: boolean
  destructive: boolean
}

interface Group {
  id: string
  label: string
  rows: Row[]
}

function groupsOf(catalogue: Catalogue, selected: string[], query: string): Group[] {
  const needle = query.trim().toLowerCase()
  const matches = (row: Row) => needle.length === 0 || row.id.toLowerCase().includes(needle)
  const groups: Group[] = catalogue.servers.map((server) => ({
    id: server.id,
    label: server.label,
    rows: catalogue.tools
      .filter((tool) => tool.server === server.id)
      .map((tool) => ({ id: toolId(tool), name: tool.name, readOnly: tool.readOnly, destructive: tool.destructive }))
      .filter(matches),
  }))
  const other = selected
    .filter((id) => findTool(catalogue, id) === undefined)
    .map((id) => ({ id, name: serverOfTool(id) ? nameOfTool(id) : id, readOnly: false, destructive: false }))
    .filter(matches)
  if (other.length > 0) groups.push({ id: 'other', label: 'Other', rows: other })
  return groups.filter((group) => group.rows.length > 0)
}

function Badge({ children, tone }: { children: React.ReactNode; tone: 'muted' | 'destructive' }) {
  return (
    <span
      className={cn(
        'shrink-0 rounded-full border px-1.5 text-[10px]',
        tone === 'destructive' ? 'border-destructive/40 bg-destructive/10 text-destructive' : 'text-muted-foreground',
      )}
    >
      {children}
    </span>
  )
}

export function ToolPicker({
  tools,
  disabled,
  onChange,
}: {
  tools: string[]
  disabled?: boolean
  onChange: (tools: string[]) => void
}) {
  const [query, setQuery] = useState('')
  const catalogue = useCatalogue()
  const groups = useMemo(() => groupsOf(catalogue, tools, query), [catalogue, tools, query])

  return (
    <div className='space-y-2'>
      <Input
        type='search'
        value={query}
        placeholder='Search tools'
        aria-label='Search tools'
        onChange={(event) => setQuery(event.target.value)}
      />
      <div className='max-h-56 space-y-2 overflow-y-auto rounded-md border p-2'>
        {groups.length === 0 && <p className='px-1 py-2 text-xs text-muted-foreground'>No tool matches.</p>}
        {groups.map((group) => (
          <div key={group.id}>
            <p className='px-1 pb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground'>
              {group.label}
            </p>
            {group.rows.map((row) => (
              <label
                key={row.id}
                className='flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1 text-sm hover:bg-accent'
              >
                <input
                  type='checkbox'
                  className='size-3.5 accent-primary'
                  checked={tools.includes(row.id)}
                  disabled={disabled}
                  onChange={() => onChange(toggleItem(tools, row.id))}
                />
                <span className='min-w-0 flex-1 truncate font-mono text-xs'>{row.name}</span>
                {row.readOnly && <Badge tone='muted'>read-only</Badge>}
                {row.destructive && <Badge tone='destructive'>destructive</Badge>}
              </label>
            ))}
          </div>
        ))}
      </div>
      <p className='text-xs text-muted-foreground'>
        {tools.length} {tools.length === 1 ? 'tool' : 'tools'} selected
      </p>
    </div>
  )
}
