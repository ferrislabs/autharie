import { FREE_OFFER, FREE_OFFER_COPY } from '../../../free-offer'
import { OFFER_COPY } from '../../../offers'
import { OptionCard } from './option-card'

export function FreeOfferSection() {
  return (
    <div className='space-y-4'>
      <OptionCard
        selected
        onSelect={() => {}}
        label={OFFER_COPY[FREE_OFFER].label}
        description={OFFER_COPY[FREE_OFFER].description}
        footer={<span className='text-xs text-muted-foreground'>{FREE_OFFER_COPY.address}</span>}
      />
    </div>
  )
}
