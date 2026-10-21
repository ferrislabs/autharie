import { CalendarClock } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState } from '@/components/layout/page'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { formatMinor } from '@/domain/billing/billing'
import { estimateRows } from './estimate-rows'

interface Props {
  estimate?: Schemas.StatementResponse
  names: ReadonlyMap<string, string>
}

export function MonthToDate({ estimate, names }: Props) {
  if (!estimate || estimate.lines.length === 0) {
    return (
      <EmptyState
        icon={<CalendarClock className='h-5 w-5' />}
        title='Nothing billed yet this month'
        description='A paid deployment appears here from the day it starts.'
      />
    )
  }

  const rows = estimateRows(estimate.lines, names)

  return (
    <div className='space-y-2'>
      <p className='text-sm text-muted-foreground'>
        Estimate up to now for {estimate.period}. The statement is final when the month closes.
      </p>
      <div className='overflow-hidden rounded-lg border'>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Deployment</TableHead>
              <TableHead className='text-right'>Amount</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.id}>
                <TableCell className='font-medium'>{row.name}</TableCell>
                <TableCell className='text-right tabular-nums'>
                  {formatMinor(row.amount)}
                </TableCell>
              </TableRow>
            ))}
            <TableRow className='bg-muted/30'>
              <TableCell className='font-medium'>Total</TableCell>
              <TableCell className='text-right font-semibold tabular-nums'>
                {formatMinor(estimate.total)}
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </div>
    </div>
  )
}
