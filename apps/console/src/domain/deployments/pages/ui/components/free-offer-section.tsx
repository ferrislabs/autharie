import { FREE_OFFER, FREE_OFFER_COPY, lockedOffers } from '../../../free-offer'
import { OFFER_COPY, type OfferAvailability } from '../../../offers'
import { OptionCard } from './option-card'

interface Props {
  catalogue: OfferAvailability[]
}

export function FreeOfferSection({ catalogue }: Props) {
  return (
    <div className='space-y-4'>
      <OptionCard
        selected
        onSelect={() => {}}
        label={OFFER_COPY[FREE_OFFER].label}
        description={OFFER_COPY[FREE_OFFER].description}
        footer={<span className='text-xs text-muted-foreground'>{FREE_OFFER_COPY.address}</span>}
      />
      <div className='grid gap-3 sm:grid-cols-2'>
        {lockedOffers(catalogue).map((offer) => (
          <OptionCard
            key={offer.label}
            selected={false}
            disabled
            onSelect={() => {}}
            label={offer.label}
            description={offer.description}
            footer={<span className='text-xs font-medium'>{offer.reason}</span>}
          />
        ))}
      </div>
    </div>
  )
}
