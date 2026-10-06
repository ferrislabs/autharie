import { cn } from '@/lib/utils'
import { priceOf, type Selection, type PlanId } from '../../../pricing/model'
import { PLAN_ORDER, PLANS } from '../../../pricing/plans'
import { describePrice } from '../../../pricing/summary'
import { OptionCard } from './option-card'

interface Props {
  selection: Selection
  onSelect: (plan: PlanId) => void
  movedNotice?: string
}

export function PlanPicker({ selection, onSelect, movedNotice }: Props) {
  const { hosting, engine, plan } = selection

  return (
    <div className='space-y-3'>
      {movedNotice && (
        <p role='status' className='rounded-md border bg-muted/30 px-3 py-2 text-sm'>
          {movedNotice}
        </p>
      )}
      <div className='grid gap-3 md:grid-cols-3'>
        {PLAN_ORDER.map((id) => {
          const copy = PLANS[hosting][id]
          const price = priceOf({ ...selection, plan: id })
          const unavailable = price.kind === 'unavailable'

          return (
            <OptionCard
              key={id}
              selected={plan === id}
              disabled={unavailable}
              onSelect={() => onSelect(id)}
              label={copy.name}
              description={copy.tagline}
              footer={
                <span className='flex flex-col gap-2'>
                  <span className='text-sm font-semibold'>
                    {unavailable
                      ? 'Up to 25,000 accounts. Beyond that, choose Business.'
                      : describePrice(price)}
                  </span>
                  <ul className='space-y-1 text-xs text-muted-foreground'>
                    {copy.features.map((feature) => {
                      const missing = feature.ferriskeyOnly && engine !== 'ferriskey'

                      return (
                        <li key={feature.text} className={cn(missing && 'line-through opacity-60')}>
                          {missing ? '× ' : '✓ '}
                          {feature.text}
                        </li>
                      )
                    })}
                  </ul>
                </span>
              }
            />
          )
        })}
      </div>
    </div>
  )
}
