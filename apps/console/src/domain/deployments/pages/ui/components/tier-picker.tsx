import type { Schemas } from '@/api/api.client'
import { formatMinor, isPaid } from '@/domain/billing/billing'
import { OptionCard } from './option-card'

interface Props {
  tiers: Schemas.TierResponse[]
  selected: string
  onSelect: (code: string) => void
}

export function TierPicker({ tiers, selected, onSelect }: Props) {
  return (
    <div role='group' aria-label='Tier' className='grid gap-3 md:grid-cols-3'>
      {tiers.map((tier) => (
        <OptionCard
          key={tier.code}
          selected={tier.code === selected}
          onSelect={() => onSelect(tier.code)}
          label={tier.name}
          footer={
            <span className='flex flex-col gap-1'>
              <span className='text-sm font-semibold'>
                {isPaid(tier) ? `${formatMinor(tier.monthly_price)} per month, excl. VAT` : 'Free'}
              </span>
              {tier.first_invoice && (
                <span className='text-xs text-muted-foreground'>
                  First invoice {formatMinor(tier.first_invoice.amount)}
                </span>
              )}
            </span>
          }
        />
      ))}
    </div>
  )
}
