import { Pencil, Plus, RefreshCw, Server, Trash2 } from 'lucide-react'
import { Section } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import { MAX_SERVERS } from '../../server-rules'
import { describeServer, toolCount } from '../../server-status'
import type { AgentServer } from '../../types'

interface Props {
  servers: AgentServer[]
  isLoading: boolean
  error?: string
  canManage: boolean
  busyId: string | null
  onAdd: () => void
  onEdit: (server: AgentServer) => void
  onRemove: (server: AgentServer) => void
  onRefresh: (server: AgentServer) => void
}

function formatTime(iso: string): string {
  const date = new Date(iso)
  return Number.isNaN(date.getTime()) ? iso : date.toISOString().replace('T', ' ').slice(0, 16)
}

function Row({
  server,
  canManage,
  busy,
  onEdit,
  onRemove,
  onRefresh,
}: {
  server: AgentServer
  canManage: boolean
  busy: boolean
  onEdit: () => void
  onRemove: () => void
  onRefresh: () => void
}) {
  const chip = describeServer(server)
  const count = toolCount(server)
  const discovery = server.discovery

  return (
    <li className='flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3'>
      <div className='min-w-0 flex-1 basis-64'>
        <div className='flex items-center gap-2'>
          <p className='truncate text-sm font-medium'>{server.name}</p>
          <code className='shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px]'>{server.slug}</code>
        </div>
        <p className='truncate font-mono text-xs text-muted-foreground' title={server.url}>
          {server.url}
        </p>
        {chip.detail && <p className='mt-1 text-xs text-muted-foreground'>{chip.detail}</p>}
      </div>

      <div className='flex shrink-0 flex-col items-start gap-1 text-xs text-muted-foreground'>
        <StatusBadge tone={chip.tone}>{chip.label}</StatusBadge>
        {count && <span>{count}</span>}
        {discovery && <span>Last discovery {formatTime(discovery.discovered_at)} UTC</span>}
      </div>

      {canManage && (
        <div className='flex shrink-0 items-center gap-1'>
          <Button
            type='button'
            variant='ghost'
            size='sm'
            disabled={busy || !server.enabled}
            aria-label={`Refresh the tools of ${server.name}`}
            onClick={onRefresh}
          >
            <RefreshCw className='size-4' />
            Refresh
          </Button>
          <Button
            type='button'
            variant='ghost'
            size='icon-sm'
            aria-label={`Edit ${server.name}`}
            onClick={onEdit}
          >
            <Pencil className='size-4' />
          </Button>
          <Button
            type='button'
            variant='ghost'
            size='icon-sm'
            aria-label={`Remove ${server.name}`}
            onClick={onRemove}
          >
            <Trash2 className='size-4' />
          </Button>
        </div>
      )}
    </li>
  )
}

export function ServersSection({
  servers,
  isLoading,
  error,
  canManage,
  busyId,
  onAdd,
  onEdit,
  onRemove,
  onRefresh,
}: Props) {
  const full = servers.length >= MAX_SERVERS

  return (
    <Section
      title='Your MCP servers'
      aside={
        canManage && (
          <Button type='button' size='sm' variant='outline' disabled={full} onClick={onAdd}>
            <Plus className='size-4' />
            Add a server
          </Button>
        )
      }
    >
      <p className='text-sm text-muted-foreground'>
        Register your own MCP server and agents reach it through this deployment, under the same
        policies and decision log as FerrisKey’s tools. It must be public over https and accept a
        FerrisKey token.
        {full && ` A deployment holds at most ${MAX_SERVERS} servers.`}
      </p>

      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      {isLoading ? (
        <Skeleton className='h-20 w-full' />
      ) : servers.length === 0 ? (
        <div className='flex items-center gap-3 rounded-lg border border-dashed p-4 text-sm text-muted-foreground'>
          <Server className='size-5 shrink-0' />
          No MCP server of yours yet.
        </div>
      ) : (
        <ul className='divide-y rounded-lg border bg-card'>
          {servers.map((server) => (
            <Row
              key={server.id}
              server={server}
              canManage={canManage}
              busy={busyId === server.id}
              onEdit={() => onEdit(server)}
              onRemove={() => onRemove(server)}
              onRefresh={() => onRefresh(server)}
            />
          ))}
        </ul>
      )}
    </Section>
  )
}
