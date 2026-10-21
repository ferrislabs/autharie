import type { Schemas } from '@/api/api.client'
import { formatMinor, isPaid } from '@/domain/billing/billing'
import type { Hosting, Price } from '../../../pricing/model'
import {
  BILLED_ON_ITS_OWN,
  INDICATIVE_NOTE,
  formatAmount,
  summaryLines,
  type SummaryLine,
} from '../../../pricing/summary'

interface Props {
  hosting: Hosting
  price: Price
  clusterEstimate?: string
  tier?: Schemas.TierResponse
}

function tierAmount(tier: Schemas.TierResponse): string {
  return isPaid(tier) ? formatMinor(tier.monthly_price) : 'Free'
}

function tierLines(tier: Schemas.TierResponse): SummaryLine[] {
  const lines: SummaryLine[] = [
    {
      label: 'Autharie',
      value: isPaid(tier) ? `${formatMinor(tier.monthly_price)} per month, excl. VAT` : 'Free',
    },
  ]

  if (tier.first_invoice) {
    lines.push({ label: 'First invoice', value: formatMinor(tier.first_invoice.amount) })
  }

  return lines
}

export function PriceSummary({ hosting, price, clusterEstimate, tier }: Props) {
  const lines = tier ? tierLines(tier) : summaryLines(hosting, price, clusterEstimate)
  const free = tier ? !isPaid(tier) : price.kind === 'price' && price.amount === 0

  return (
    <div aria-live='polite' className='space-y-2 rounded-lg border bg-muted/30 p-4'>
      <div className='flex items-baseline justify-between gap-4'>
        <span className='text-sm font-medium'>Monthly price</span>
        <span className='text-xl font-semibold tabular-nums'>
          {tier ? tierAmount(tier) : formatAmount(price)}
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
      {!tier && <p className='text-xs text-muted-foreground'>{INDICATIVE_NOTE}</p>}
    </div>
  )
}
