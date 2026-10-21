import { format } from 'date-fns'
import type { Schemas } from '@/api/api.client'
import { Card } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import {
  NEEDS_BILLING_RIGHT,
  fittingTiers,
  formatMinor,
  isPaid,
  paidActionBlock,
  type BillingAccess,
  type TierFit,
} from '@/domain/billing/billing'

export interface SubscriptionCardProps {
  subscription?: Schemas.SubscriptionResponse
  isLoading: boolean
  isError: boolean
  tiers?: ReadonlyArray<Schemas.TierResponse>
  fit: TierFit
  access: BillingAccess
  changing: string | null
  error?: string
  onChange: (tier: string) => void
}

function priceLabel(monthlyPrice: number): string {
  return isPaid({ monthly_price: monthlyPrice }) ? `${formatMinor(monthlyPrice)} per month` : 'Free'
}

function changeBlock(tier: Schemas.TierResponse, access: BillingAccess): string | null {
  if (!access.canManage) return NEEDS_BILLING_RIGHT
  return isPaid(tier) ? paidActionBlock(access) : null
}

function TierRow({
  tier,
  block,
  changing,
  onChange,
}: {
  tier: Schemas.TierResponse
  block: string | null
  changing: string | null
  onChange: (tier: string) => void
}) {
  const reason = `tier-${tier.code}-block`

  return (
    <li className='space-y-1 py-3'>
      <div className='flex items-center justify-between gap-4'>
        <div className='min-w-0'>
          <p className='truncate font-medium'>{tier.name}</p>
          <p className='text-xs text-muted-foreground'>{priceLabel(tier.monthly_price)}</p>
        </div>
        <Button
          size='sm'
          variant='outline'
          disabled={block !== null || changing !== null}
          aria-describedby={block ? reason : undefined}
          onClick={() => onChange(tier.code)}
        >
          {changing === tier.code && <Spinner className='size-3.5' />}
          {changing === tier.code ? 'Switching' : 'Switch'}
        </Button>
      </div>
      {block && (
        <p id={reason} className='text-xs text-muted-foreground'>
          {block}
        </p>
      )}
    </li>
  )
}

function TierChange({
  current,
  tiers,
  fit,
  access,
  changing,
  error,
  onChange,
}: {
  current: string
  tiers?: ReadonlyArray<Schemas.TierResponse>
  fit: TierFit
  access: BillingAccess
  changing: string | null
  error?: string
  onChange: (tier: string) => void
}) {
  if (!tiers) return null

  if (tiers.length === 0) {
    return (
      <p className='mt-4 border-t pt-4 text-sm text-muted-foreground'>
        The tier catalogue is not open yet.
      </p>
    )
  }

  const others = fittingTiers(tiers, fit).filter((tier) => tier.code !== current)

  if (others.length === 0) {
    return (
      <p className='mt-4 border-t pt-4 text-sm text-muted-foreground'>No other tier available.</p>
    )
  }

  return (
    <div className='mt-4 border-t pt-4'>
      <p className='text-sm font-medium'>Change tier</p>
      <ul className='divide-y'>
        {others.map((tier) => (
          <TierRow
            key={tier.code}
            tier={tier}
            block={changeBlock(tier, access)}
            changing={changing}
            onChange={onChange}
          />
        ))}
      </ul>
      {error && (
        <p role='alert' className='mt-2 text-sm text-destructive'>
          {error}
        </p>
      )}
    </div>
  )
}

export function SubscriptionCard({
  subscription,
  isLoading,
  isError,
  tiers,
  fit,
  access,
  changing,
  error,
  onChange,
}: SubscriptionCardProps) {
  return (
    <Card className='p-4'>
      <p className='text-sm text-muted-foreground'>Subscription</p>
      {isLoading && <Skeleton className='mt-2 h-8 w-40' />}
      {!isLoading && !subscription && (
        <p className='mt-1 text-sm text-muted-foreground'>
          {isError ? 'The subscription could not be loaded.' : 'No subscription yet.'}
        </p>
      )}
      {subscription && (
        <>
          <div className='mt-1 flex flex-wrap items-baseline gap-x-8 gap-y-2'>
            <div>
              <p className='text-lg font-semibold'>{subscription.tier}</p>
              <p className='text-xs text-muted-foreground'>
                Since {format(new Date(subscription.starts_at), 'd MMM yyyy')}
              </p>
            </div>
            <p className='text-lg font-semibold'>{priceLabel(subscription.monthly_price)}</p>
          </div>
          <TierChange
            current={subscription.tier}
            tiers={tiers}
            fit={fit}
            access={access}
            changing={changing}
            error={error}
            onChange={onChange}
          />
        </>
      )}
    </Card>
  )
}
