import { ScrollText } from 'lucide-react'
import { EmptyState, Section } from '@/components/layout/page'
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import {
  RANGES,
  awaitsApproval,
  decisionBadge,
  decisionKey,
  emptyFilters,
  hasFilters,
  policyLabel,
  reasonLabel,
  subjectLabel,
  type DecisionFilters,
  type DecisionRange,
} from '../../decisions'
import type { AgentDecision, AgentPolicy } from '../../types'

interface Props {
  decisions: AgentDecision[]
  policies: AgentPolicy[]
  isLoading: boolean
  error?: string
  filters: DecisionFilters
  onFilters: (filters: DecisionFilters) => void
  onApprove?: (clientId: string) => void
}

function formatTime(iso: string): string {
  const date = new Date(iso)
  return Number.isNaN(date.getTime()) ? iso : date.toISOString().replace('T', ' ').slice(0, 19)
}

export function DecisionLog({
  decisions,
  policies,
  isLoading,
  error,
  filters,
  onFilters,
  onApprove,
}: Props) {
  const filtered = hasFilters(filters)

  return (
    <Section
      title='Decision log'
      aside={
        <span className='text-xs text-muted-foreground'>
          Refreshes every 5 seconds · times in UTC
        </span>
      }
    >
      <div className='flex flex-wrap items-center gap-2'>
        <Select
          value={filters.decision}
          onValueChange={(decision) =>
            onFilters({ ...filters, decision: decision as DecisionFilters['decision'] })
          }
        >
          <SelectTrigger className='w-36' aria-label='Decision'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='all'>All decisions</SelectItem>
            <SelectItem value='allow'>Allow</SelectItem>
            <SelectItem value='deny'>Deny</SelectItem>
          </SelectContent>
        </Select>
        <Input
          className='w-56'
          placeholder='Tool, e.g. autharie/list_backups'
          aria-label='Tool'
          value={filters.tool}
          onChange={(event) => onFilters({ ...filters, tool: event.target.value })}
        />
        <Input
          className='w-48'
          placeholder='Subject'
          aria-label='Subject'
          value={filters.subject}
          onChange={(event) => onFilters({ ...filters, subject: event.target.value })}
        />
        <Input
          className='w-48'
          placeholder='Agent (client id)'
          aria-label='Agent'
          value={filters.agent}
          onChange={(event) => onFilters({ ...filters, agent: event.target.value })}
        />
        <Select
          value={filters.range}
          onValueChange={(range) => onFilters({ ...filters, range: range as DecisionRange })}
        >
          <SelectTrigger className='w-40' aria-label='Period'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {RANGES.map((range) => (
              <SelectItem key={range.id} value={range.id}>
                {range.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {filtered && (
          <Button variant='ghost' size='sm' onClick={() => onFilters(emptyFilters())}>
            Clear filters
          </Button>
        )}
      </div>

      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      {!isLoading && decisions.length === 0 ? (
        <EmptyState
          icon={<ScrollText className='h-5 w-5' />}
          title={filtered ? 'No decision matches these filters' : 'No decisions yet'}
          description={
            filtered
              ? 'Widen the period or clear a filter.'
              : 'Every tool call an agent makes through the gateway is recorded here, allowed or not.'
          }
        />
      ) : (
        <div className='overflow-x-auto rounded-lg border'>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Time</TableHead>
                <TableHead>Decision</TableHead>
                <TableHead>Tool</TableHead>
                <TableHead>Subject</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>Delegated</TableHead>
                <TableHead>Policies</TableHead>
                <TableHead>Reason</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {decisions.map((decision, index) => {
                const badge = decisionBadge(decision.decision)

                return (
                  <TableRow key={decisionKey(decision, index)}>
                    <TableCell className='whitespace-nowrap font-mono text-xs'>
                      {formatTime(decision.time)}
                    </TableCell>
                    <TableCell>
                      <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>
                    </TableCell>
                    <TableCell className='font-mono text-xs'>
                      {decision.tool ?? decision.method}
                    </TableCell>
                    <TableCell className='font-mono text-xs'>{subjectLabel(decision)}</TableCell>
                    <TableCell className='font-mono text-xs'>{decision.agent}</TableCell>
                    <TableCell>{decision.delegated ? 'Yes' : 'No'}</TableCell>
                    <TableCell className='font-mono text-xs'>
                      {decision.policies.length === 0
                        ? 'None'
                        : decision.policies.map((name) => policyLabel(name, policies)).join(', ')}
                    </TableCell>
                    <TableCell className='max-w-xs whitespace-normal text-xs text-muted-foreground'>
                      <span>{reasonLabel(decision.reason)}</span>
                      {onApprove && awaitsApproval(decision) && (
                        <Button
                          size='sm'
                          variant='outline'
                          className='ml-2'
                          onClick={() => onApprove(decision.agent)}
                        >
                          Approve
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </Section>
  )
}
