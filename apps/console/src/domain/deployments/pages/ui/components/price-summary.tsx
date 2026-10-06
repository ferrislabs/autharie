import type { Hosting, Price } from '../../../pricing/model'
import {
  BILLED_ON_ITS_OWN,
  INDICATIVE_NOTE,
  formatAmount,
  summaryLines,
} from '../../../pricing/summary'

interface Props {
  hosting: Hosting
  price: Price
  clusterEstimate?: string
}

export function PriceSummary({ hosting, price, clusterEstimate }: Props) {
  const lines = summaryLines(hosting, price, clusterEstimate)
  const free = price.kind === 'price' && price.amount === 0

  return (
    <div aria-live='polite' className='space-y-2 rounded-lg border bg-muted/30 p-4'>
      <div className='flex items-baseline justify-between gap-4'>
        <span className='text-sm font-medium'>Monthly price</span>
        <span className='text-xl font-semibold tabular-nums'>
          {formatAmount(price)}
          {!free && <span className='text-xs font-normal text-muted-foreground'> excl. VAT</span>}
        </span>
      </div>
      <dl className='space-y-1 text-sm'>
        {lines.map((line) => (
          <div key={line.label} className='flex justify-between gap-4'>
            <dt className='text-muted-foreground'>{line.label}</dt>
            <dd className='text-right'>{line.value}</dd>
          </div>
        ))}
      </dl>
      <p className='text-xs text-muted-foreground'>{BILLED_ON_ITS_OWN}</p>
      <p className='text-xs text-muted-foreground'>{INDICATIVE_NOTE}</p>
    </div>
  )
}
