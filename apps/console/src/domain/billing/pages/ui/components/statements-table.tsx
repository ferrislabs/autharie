import { Receipt } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState } from '@/components/layout/page'
import { StatusBadge, type Tone } from '@/components/ui/status-badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { formatMinor, payNowUrl } from '@/domain/billing/billing'

const STATUSES: Record<Schemas.StatementStatus, { label: string; tone: Tone }> = {
  zero: { label: 'Nothing to pay', tone: 'neutral' },
  held: { label: 'On hold', tone: 'neutral' },
  draft: { label: 'Being prepared', tone: 'neutral' },
  sent: { label: 'Awaiting payment', tone: 'warning' },
  paid: { label: 'Paid', tone: 'success' },
  payment_failed: { label: 'Payment failed', tone: 'danger' },
  uncollectible: { label: 'Uncollectible', tone: 'danger' },
}

interface Props {
  statements: ReadonlyArray<Schemas.StatementSummary>
}

export function StatementsTable({ statements }: Props) {
  if (statements.length === 0) {
    return (
      <EmptyState
        icon={<Receipt className='h-5 w-5' />}
        title='No statement yet'
        description='A statement appears here when a month closes.'
      />
    )
  }

  return (
    <div className='overflow-hidden rounded-lg border'>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Period</TableHead>
            <TableHead>Status</TableHead>
            <TableHead className='text-right'>Total</TableHead>
            <TableHead />
          </TableRow>
        </TableHeader>
        <TableBody>
          {statements.map((statement) => {
            const url = payNowUrl(statement)
            const { label, tone } = STATUSES[statement.status]

            return (
              <TableRow key={statement.period}>
                <TableCell className='font-medium'>{statement.period}</TableCell>
                <TableCell>
                  <StatusBadge tone={tone}>{label}</StatusBadge>
                </TableCell>
                <TableCell className='text-right tabular-nums'>
                  {formatMinor(statement.total)}
                </TableCell>
                <TableCell className='text-right'>
                  {url && (
                    <a
                      href={url}
                      target='_blank'
                      rel='noopener'
                      className='text-sm font-medium text-primary underline-offset-4 hover:underline'
                    >
                      Pay now
                    </a>
                  )}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}
