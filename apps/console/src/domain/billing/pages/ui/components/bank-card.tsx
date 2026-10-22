import { CreditCard } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { brandLabel, formatExpiry, maskedNumber } from '@/domain/billing/billing'

const SHAPE = 'aspect-[85.6/53.98] w-full max-w-sm rounded-xl'

export function BankCard({ card }: { card: Schemas.CardResponse }) {
  const brand = brandLabel(card.brand)
  const expiry = formatExpiry(card)

  return (
    <div
      role='img'
      aria-label={`${brand} card ending in ${card.last4}, expires ${expiry}`}
      className={`${SHAPE} relative flex flex-col justify-between overflow-hidden bg-gradient-to-br from-slate-600 via-slate-800 to-slate-950 p-5 text-white shadow-md`}
    >
      <span aria-hidden='true' className='absolute -right-12 -top-12 size-44 rounded-full bg-white/5' />
      <span aria-hidden='true' className='absolute -bottom-16 -left-8 size-48 rounded-full bg-white/5' />
      <div aria-hidden='true' className='relative flex items-start justify-between'>
        <span className='h-8 w-11 rounded-md bg-gradient-to-br from-amber-200 via-amber-300 to-amber-500 shadow-inner' />
        <span className='text-sm font-semibold italic tracking-wide'>{brand}</span>
      </div>
      <p aria-hidden='true' className='relative whitespace-nowrap font-mono text-base tracking-[0.14em] sm:text-xl sm:tracking-[0.18em]'>
        {maskedNumber(card)}
      </p>
      <div aria-hidden='true' className='relative'>
        <span className='block text-[10px] uppercase tracking-wider text-white/60'>Valid thru</span>
        <span className='font-mono text-sm'>{expiry}</span>
      </div>
    </div>
  )
}

export function BankCardPlaceholder({ message }: { message: string }) {
  return (
    <div
      className={`${SHAPE} flex flex-col items-center justify-center gap-2 border border-dashed bg-muted/30 text-sm text-muted-foreground`}
    >
      <CreditCard className='h-5 w-5' />
      {message}
    </div>
  )
}
