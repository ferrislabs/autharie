import { Inbox } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { DELIVERY_FILTERS, canRetry, deliveryBadge, relativeTime, truncate } from '../../deliveries'
import { Hint } from './hint'

interface Props {
  isLoading: boolean
  error?: string
  deliveries: Schemas.WebhookDelivery[]
  filter: string
  onFilter: (filter: string) => void
  requested: ReadonlySet<string>
  canManage: boolean
  now: number
  onRetry: (deliveryId: string) => void
}

export function DeliveriesTable({
  isLoading,
  error,
  deliveries,
  filter,
  onFilter,
  requested,
  canManage,
  now,
  onRetry,
}: Props) {
  return (
    <section className='space-y-3'>
      <div className='flex items-center justify-between gap-4'>
        <h2 className='text-base font-semibold'>Deliveries</h2>
        <Select value={filter} onValueChange={onFilter}>
          <SelectTrigger className='w-44' aria-label='Filter by status'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {DELIVERY_FILTERS.map(({ value, label }) => (
              <SelectItem key={value} value={value}>
                {label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      {isLoading ? (
        <Skeleton className='h-32 w-full' />
      ) : deliveries.length === 0 ? (
        <EmptyState
          icon={<Inbox className='h-5 w-5' />}
          title={filter === 'all' ? 'No deliveries yet' : 'No delivery with this status'}
          description={
            filter === 'all'
              ? 'They appear here a few seconds after the first event happens. This list refreshes by itself.'
              : 'Choose another status to see the rest.'
          }
        />
      ) : (
        <div className='overflow-hidden rounded-lg border'>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Status</TableHead>
                <TableHead>Event</TableHead>
                <TableHead>Attempts</TableHead>
                <TableHead>Last response</TableHead>
                <TableHead>Last error</TableHead>
                <TableHead>When</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {deliveries.map((delivery) => (
                <DeliveryRow
                  key={delivery.delivery_id}
                  delivery={delivery}
                  requested={requested.has(delivery.delivery_id)}
                  canManage={canManage}
                  now={now}
                  onRetry={onRetry}
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  )
}

function DeliveryRow({
  delivery,
  requested,
  canManage,
  now,
  onRetry,
}: {
  delivery: Schemas.WebhookDelivery
  requested: boolean
  canManage: boolean
  now: number
  onRetry: (deliveryId: string) => void
}) {
  const badge = deliveryBadge(delivery.status)

  return (
    <TableRow>
      <TableCell>
        <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>
      </TableCell>
      <TableCell className='font-mono text-xs'>{delivery.event}</TableCell>
      <TableCell>{delivery.attempt_count}</TableCell>
      <TableCell className='font-mono text-xs'>{delivery.last_status_code ?? '-'}</TableCell>
      <TableCell className='max-w-56 text-xs text-muted-foreground'>
        {delivery.last_error ? (
          <Hint tip={delivery.last_error}>{truncate(delivery.last_error, 48)}</Hint>
        ) : (
          '-'
        )}
      </TableCell>
      <TableCell className='whitespace-nowrap text-xs text-muted-foreground'>
        <Hint tip={new Date(delivery.created_at).toLocaleString()}>
          {relativeTime(delivery.created_at, now)}
        </Hint>
      </TableCell>
      <TableCell className='text-right'>
        {canManage &&
          canRetry(delivery) &&
          (requested ? (
            <span className='text-xs text-muted-foreground'>Retry requested</span>
          ) : (
            <Button variant='outline' size='sm' onClick={() => onRetry(delivery.delivery_id)}>
              Retry
            </Button>
          ))}
      </TableCell>
    </TableRow>
  )
}
