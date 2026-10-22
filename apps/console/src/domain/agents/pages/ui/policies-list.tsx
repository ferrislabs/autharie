import { Pencil, Plus, ShieldCheck, Trash2 } from 'lucide-react'
import { EmptyState, Section } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { StatusBadge, type Tone } from '@/components/ui/status-badge'
import { Switch } from '@/components/ui/switch'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { summarisePolicy } from '../../summary'
import type { AgentPolicy, PolicyPhase } from '../../types'

interface Props {
  policies: AgentPolicy[]
  isLoading: boolean
  error?: string
  canManage: boolean
  busyId: string | null
  onCreate: () => void
  onEdit: (policy: AgentPolicy) => void
  onDelete: (policy: AgentPolicy) => void
  onToggle: (policy: AgentPolicy, enabled: boolean) => void
}

const PHASES: Record<PolicyPhase, { tone: Tone; label: string }> = {
  Pending: { tone: 'progress', label: 'Pending' },
  Applied: { tone: 'success', label: 'Applied' },
  Invalid: { tone: 'danger', label: 'Invalid' },
  Failed: { tone: 'danger', label: 'Failed' },
}

export function PolicyPhaseBadge({ policy }: { policy: AgentPolicy }) {
  if (!policy.enabled) return <StatusBadge tone='neutral'>Not applied</StatusBadge>
  const phase = PHASES[policy.phase ?? 'Pending']

  return (
    <div className='space-y-1'>
      <StatusBadge tone={phase.tone}>{phase.label}</StatusBadge>
      {policy.message && <p className='max-w-xs text-xs text-muted-foreground'>{policy.message}</p>}
    </div>
  )
}

export function PoliciesList({
  policies,
  isLoading,
  error,
  canManage,
  busyId,
  onCreate,
  onEdit,
  onDelete,
  onToggle,
}: Props) {
  return (
    <Section
      title='Policies'
      aside={
        canManage && (
          <Button size='sm' onClick={onCreate}>
            <Plus className='h-3.5 w-3.5' />
            New policy
          </Button>
        )
      }
    >
      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      {!isLoading && policies.length === 0 ? (
        <EmptyState
          icon={<ShieldCheck className='h-5 w-5' />}
          title='No policies yet'
          description='Without a permit policy every agent call is denied. Write a rule to say which agent may call which tool, for whom and when.'
          action={
            canManage ? (
              <Button onClick={onCreate}>
                <Plus className='h-3.5 w-3.5' />
                Create your first policy
              </Button>
            ) : (
              <p className='text-sm text-muted-foreground'>
                Ask someone who can manage instances to create one.
              </p>
            )
          }
        />
      ) : (
        <div className='overflow-hidden rounded-lg border'>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Rule</TableHead>
                <TableHead>Enabled</TableHead>
                <TableHead>State</TableHead>
                {canManage && <TableHead className='w-24' />}
              </TableRow>
            </TableHeader>
            <TableBody>
              {policies.map((policy) => (
                <TableRow key={policy.id}>
                  <TableCell className='font-medium'>
                    {policy.name}
                    {policy.description && (
                      <p className='text-xs font-normal text-muted-foreground'>{policy.description}</p>
                    )}
                  </TableCell>
                  <TableCell className='max-w-md whitespace-normal text-sm'>
                    {summarisePolicy(policy)}
                  </TableCell>
                  <TableCell>
                    <Switch
                      checked={policy.enabled}
                      disabled={!canManage || busyId === policy.id}
                      aria-label={`${policy.enabled ? 'Disable' : 'Enable'} ${policy.name}`}
                      onCheckedChange={(enabled) => onToggle(policy, enabled)}
                    />
                  </TableCell>
                  <TableCell>
                    <PolicyPhaseBadge policy={policy} />
                  </TableCell>
                  {canManage && (
                    <TableCell>
                      <div className='flex justify-end gap-1'>
                        <Button
                          variant='ghost'
                          size='icon-sm'
                          aria-label={`Edit ${policy.name}`}
                          onClick={() => onEdit(policy)}
                        >
                          <Pencil className='h-3.5 w-3.5' />
                        </Button>
                        <Button
                          variant='ghost'
                          size='icon-sm'
                          aria-label={`Delete ${policy.name}`}
                          onClick={() => onDelete(policy)}
                        >
                          <Trash2 className='h-3.5 w-3.5' />
                        </Button>
                      </div>
                    </TableCell>
                  )}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </Section>
  )
}
