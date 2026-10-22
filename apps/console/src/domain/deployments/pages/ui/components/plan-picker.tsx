import { Check, Minus, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { priceOf, type Selection, type PlanId } from '../../../pricing/model'
import { FEATURES, PLAN_ORDER, PLANS, type Cell } from '../../../pricing/plans'
import { describePrice } from '../../../pricing/summary'
import { OptionCard } from './option-card'

interface Props {
  selection: Selection
  onSelect: (plan: PlanId) => void
  movedNotice?: string
  closedPlans?: PlanId[]
}

function CellValue({ cell, missing }: { cell: Cell; missing: boolean }) {
  if (missing) {
    return (
      <>
        <X aria-hidden='true' className='h-4 w-4 text-muted-foreground' />
        <span className='sr-only'>Not available with this engine</span>
      </>
    )
  }
  if (cell === true) {
    return (
      <>
        <Check aria-hidden='true' className='h-4 w-4 text-primary' />
        <span className='sr-only'>Included</span>
      </>
    )
  }
  if (cell === false) {
    return (
      <>
        <Minus aria-hidden='true' className='h-4 w-4 text-muted-foreground' />
        <span className='sr-only'>Not included</span>
      </>
    )
  }
  return <span>{cell}</span>
}

export function PlanPicker({ selection, onSelect, movedNotice, closedPlans = [] }: Props) {
  const { hosting, engine, plan } = selection

  return (
    <div className='space-y-3'>
      {movedNotice && (
        <p role='status' className='rounded-md border bg-muted/30 px-3 py-2 text-sm'>
          {movedNotice}
        </p>
      )}
      <div className='overflow-x-auto'>
        <table className='w-full min-w-[42rem] table-fixed border-collapse text-sm'>
          <thead>
            <tr>
              <th scope='col' className='w-1/4'>
                <span className='sr-only'>Feature</span>
              </th>
              {PLAN_ORDER.map((id) => {
                const copy = PLANS[hosting][id]
                const price = priceOf({ ...selection, plan: id })
                const closed = closedPlans.includes(id)
                const unavailable = price.kind === 'unavailable'

                return (
                  <th key={id} scope='col' className='h-px p-1.5 align-top font-normal'>
                    <OptionCard
                      className='h-full w-full'
                      selected={plan === id}
                      disabled={unavailable || closed}
                      onSelect={() => onSelect(id)}
                      label={copy.name}
                      description={copy.tagline}
                      footer={
                        <span className='text-sm font-semibold'>
                          {closed
                            ? 'Not available on your plan.'
                            : unavailable
                              ? 'Up to 25,000 accounts. Beyond that, choose Business.'
                              : describePrice(price)}
                        </span>
                      }
                    />
                  </th>
                )
              })}
            </tr>
          </thead>
          <tbody>
            {FEATURES[hosting].map((row) => {
              const missing = row.ferriskeyOnly === true && engine !== 'ferriskey'

              return (
                <tr key={row.label} className='border-t'>
                  <th scope='row' className='py-3 pr-4 text-left align-top font-medium'>
                    {row.label}
                    {row.hint && (
                      <span className='mt-0.5 block text-xs font-normal text-muted-foreground'>
                        {row.hint}
                      </span>
                    )}
                  </th>
                  {PLAN_ORDER.map((id) => (
                    <td
                      key={id}
                      className={cn(
                        'px-3.5 py-3 align-top',
                        typeof row.cells[id] === 'string' && !missing && 'text-muted-foreground',
                        plan === id && 'bg-primary/5',
                      )}
                    >
                      <CellValue cell={row.cells[id]} missing={missing} />
                    </td>
                  ))}
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}
