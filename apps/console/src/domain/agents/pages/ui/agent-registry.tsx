import { Bot, KeyRound, Plus } from 'lucide-react'
import { EmptyState, Section } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import { Switch } from '@/components/ui/switch'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { cn } from '@/lib/utils'
import {
  ACCESS_BADGE,
  SOURCE_LABEL,
  blockedCount,
  canApprove,
  canBlock,
  canDelete,
  canRotate,
  canUnblock,
  displayName,
  filterClients,
  kindLabel,
  lastSeenLabel,
  pendingCount,
  type RegistryFilter,
} from '../../registry'
import type { AgentClient, Registry } from '../../types'

interface Props {
  registry?: Registry
  isLoading: boolean
  error?: string
  canManage: boolean
  filter: RegistryFilter
  approvalSaving: boolean
  busyClient: string | null
  now: Date
  onFilter: (filter: RegistryFilter) => void
  onApprovalRequired: (required: boolean) => void
  onApprove: (client: AgentClient) => void
  onUnblock: (client: AgentClient) => void
  onBlock: (client: AgentClient) => void
  onDelete: (client: AgentClient) => void
  onRotate: (client: AgentClient) => void
  onNewAutonomous: () => void
}

function FilterChip({
  active,
  label,
  count,
  onClick,
}: {
  active: boolean
  label: string
  count: number
  onClick: () => void
}) {
  return (
    <button
      type='button'
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium transition-colors',
        active
          ? 'border-primary/40 bg-primary/10 text-primary'
          : 'text-muted-foreground hover:bg-muted',
      )}
    >
      {label}
      <span className='tabular-nums'>{count}</span>
    </button>
  )
}

export function AgentRegistry({
  registry,
  isLoading,
  error,
  canManage,
  filter,
  approvalSaving,
  busyClient,
  now,
  onFilter,
  onApprovalRequired,
  onApprove,
  onUnblock,
  onBlock,
  onDelete,
  onRotate,
  onNewAutonomous,
}: Props) {
  if (isLoading) return <Skeleton className='h-48 w-full' />

  const clients = registry?.data ?? []
  const shown = filterClients(clients, filter)

  return (
    <div className='space-y-5'>
      <div className='flex items-center justify-between gap-4 rounded-lg border bg-card p-5'>
        <div className='space-y-0.5'>
          <Label htmlFor='approval-required' className='text-base font-semibold'>
            Require approval for new agents
          </Label>
          <p className='text-sm text-muted-foreground'>
            When on, an agent is refused at the gateway until you approve it; blocked agents are
            always refused.
          </p>
        </div>
        <Switch
          id='approval-required'
          checked={registry?.approval_required ?? false}
          disabled={!canManage || approvalSaving || !registry}
          onCheckedChange={onApprovalRequired}
        />
      </div>

      <Section
        title='Agents'
        aside={
          canManage && (
            <Button size='sm' onClick={onNewAutonomous}>
              <Plus className='h-4 w-4' />
              New autonomous agent
            </Button>
          )
        }
      >
        <div className='flex flex-wrap items-center gap-2'>
          <FilterChip
            active={filter === 'all'}
            label='All'
            count={clients.length}
            onClick={() => onFilter('all')}
          />
          <FilterChip
            active={filter === 'pending'}
            label='Pending'
            count={pendingCount(clients)}
            onClick={() => onFilter('pending')}
          />
          <FilterChip
            active={filter === 'blocked'}
            label='Blocked'
            count={blockedCount(clients)}
            onClick={() => onFilter('blocked')}
          />
        </div>

        {error && (
          <p role='alert' className='text-sm text-destructive'>
            {error}
          </p>
        )}

        {registry?.truncated && (
          <p className='text-xs text-muted-foreground'>
            This realm has more agents than the 500 newest shown here.
          </p>
        )}

        {shown.length === 0 ? (
          <EmptyState
            icon={<Bot className='h-5 w-5' />}
            title={clients.length === 0 ? 'No agent yet' : 'No agent matches this filter'}
            description={
              clients.length === 0
                ? 'Agents appear here once they register with this deployment’s realm or make a call.'
                : 'Pick another filter to see the rest.'
            }
          />
        ) : (
          <div className='overflow-x-auto rounded-lg border'>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Agent</TableHead>
                  <TableHead>Source</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Access</TableHead>
                  <TableHead>Last activity</TableHead>
                  {canManage && <TableHead className='text-right'>Actions</TableHead>}
                </TableRow>
              </TableHeader>
              <TableBody>
                {shown.map((client) => {
                  const badge = ACCESS_BADGE[client.access]
                  const busy = busyClient === client.client_id
                  const name = displayName(client)

                  return (
                    <TableRow key={client.client_id}>
                      <TableCell className='max-w-xs'>
                        <div className='truncate font-medium'>{name}</div>
                        {name !== client.client_id && (
                          <div className='truncate font-mono text-xs text-muted-foreground'>
                            {client.client_id}
                          </div>
                        )}
                      </TableCell>
                      <TableCell>
                        <StatusBadge tone='neutral' dot={false}>
                          {SOURCE_LABEL[client.source]}
                        </StatusBadge>
                      </TableCell>
                      <TableCell>{kindLabel(client)}</TableCell>
                      <TableCell>
                        <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>
                      </TableCell>
                      <TableCell className='whitespace-nowrap text-sm text-muted-foreground'>
                        {lastSeenLabel(client.last_seen_at, now)}
                      </TableCell>
                      {canManage && (
                        <TableCell>
                          <div className='flex justify-end gap-1.5'>
                            {canApprove(client) && (
                              <Button
                                size='sm'
                                variant='outline'
                                disabled={busy}
                                onClick={() => onApprove(client)}
                              >
                                Approve
                              </Button>
                            )}
                            {canUnblock(client) && (
                              <Button
                                size='sm'
                                variant='outline'
                                disabled={busy}
                                onClick={() => onUnblock(client)}
                              >
                                Unblock
                              </Button>
                            )}
                            {canBlock(client) && (
                              <Button
                                size='sm'
                                variant='outline'
                                disabled={busy}
                                onClick={() => onBlock(client)}
                              >
                                Block
                              </Button>
                            )}
                            {canRotate(client) && (
                              <Button
                                size='sm'
                                variant='outline'
                                disabled={busy}
                                aria-label={`Rotate the secret of ${name}`}
                                onClick={() => onRotate(client)}
                              >
                                <KeyRound className='h-3.5 w-3.5' />
                                Rotate secret
                              </Button>
                            )}
                            {canDelete(client) && (
                              <Button
                                size='sm'
                                variant='ghost'
                                className='text-destructive hover:text-destructive'
                                disabled={busy}
                                onClick={() => onDelete(client)}
                              >
                                Delete
                              </Button>
                            )}
                          </div>
                        </TableCell>
                      )}
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          </div>
        )}
      </Section>
    </div>
  )
}
