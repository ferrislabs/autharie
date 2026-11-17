import { Skeleton } from '@/components/ui/skeleton'
import { OFFER_CHOICES, OFFER_CHOICE_LABELS, offerChoiceOf, openLabel } from '../../offer-override'
import type { OfferChoice, OfferName, OfferOverrideView } from '../../types'

interface Props {
  offers: OfferOverrideView[]
  isLoading: boolean
  canChange: boolean
  pending?: OfferName
  refusal?: string
  onChoose: (offer: OfferName, choice: OfferChoice) => void
}

export function OfferOverrides({
  offers,
  isLoading,
  canChange,
  pending,
  refusal,
  onChoose,
}: Props) {
  return (
    <section className='mt-10'>
      <h2 className='text-lg font-semibold'>Offers</h2>
      <p className='mt-1 text-sm text-muted-foreground'>
        Which offers this organisation may pick, whatever the plan says.
      </p>

      {refusal && (
        <div
          className='mt-4 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm'
          role='alert'
        >
          {refusal}
        </div>
      )}

      <div className='mt-4 divide-y rounded-lg border'>
        {isLoading ? (
          <Skeleton className='h-16' />
        ) : (
          offers.map((entry) => (
            <div
              key={entry.offer}
              className='flex flex-wrap items-center justify-between gap-4 px-4 py-3'
            >
              <div className='min-w-0'>
                <p className='font-mono text-sm font-medium'>{entry.offer}</p>
                <p className='mt-1 text-xs text-muted-foreground'>
                  Plan default: {openLabel(entry.plan_default)}
                </p>
              </div>
              <div className='flex items-center gap-3'>
                <select
                  className='h-8 rounded-md border bg-background px-2 text-sm'
                  aria-label={`Override for ${entry.offer}`}
                  value={offerChoiceOf(entry.override)}
                  disabled={!canChange || pending === entry.offer}
                  onChange={(event) => onChoose(entry.offer, event.target.value as OfferChoice)}
                >
                  {OFFER_CHOICES.map((choice) => (
                    <option key={choice} value={choice}>
                      {OFFER_CHOICE_LABELS[choice]}
                    </option>
                  ))}
                </select>
                <span
                  className={
                    entry.effective
                      ? 'rounded-full border border-primary/30 bg-primary/5 px-2 py-0.5 text-xs font-medium'
                      : 'rounded-full border px-2 py-0.5 text-xs font-medium text-muted-foreground'
                  }
                >
                  {openLabel(entry.effective)}
                </span>
              </div>
            </div>
          ))
        )}
      </div>
      <p className='mt-2 text-xs text-muted-foreground'>
        A paid offer opened here is billed at 0 € until paid tiers exist.
      </p>
    </section>
  )
}
