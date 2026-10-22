import { CreditCard } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { StatusBadge, type Tone } from '@/components/ui/status-badge'
import { NEEDS_BILLING_DETAILS } from '@/domain/billing/billing'
import { BankCard, BankCardPlaceholder } from './bank-card'

export const NEEDS_MANAGE_TO_ADD = 'Adding a payment method needs the Manage billing permission.'
export const NO_PAYMENT_METHOD = 'No payment method yet'
export const NO_CARD_DETAILS = 'No card details to show'

const STATUSES: Record<Schemas.PaymentMethodStatus, { label: string; tone: Tone }> = {
  missing: { label: 'Missing', tone: 'warning' },
  valid: { label: 'Valid', tone: 'success' },
  failing: { label: 'Failing', tone: 'danger' },
}

interface Props {
  status?: Schemas.PaymentMethodStatus
  card?: Schemas.CardResponse | null
  hasAccount: boolean
  canManage: boolean
  isOpening: boolean
  refusal?: string
  onAdd: () => void
}

function blockedBy(canManage: boolean, hasAccount: boolean): string | null {
  if (!canManage) return NEEDS_MANAGE_TO_ADD
  if (!hasAccount) return NEEDS_BILLING_DETAILS
  return null
}

export function PaymentMethodPanel({
  status,
  card,
  hasAccount,
  canManage,
  isOpening,
  refusal,
  onAdd,
}: Props) {
  const block = blockedBy(canManage, hasAccount)
  const current = STATUSES[status ?? 'missing']

  return (
    <div className='space-y-4 rounded-lg border bg-card p-5'>
      <div className='flex items-center justify-between gap-4'>
        <div className='flex items-center gap-2 text-sm'>
          <span className='text-muted-foreground'>Payment method</span>
          <StatusBadge tone={current.tone}>{current.label}</StatusBadge>
        </div>
        <Button
          size='sm'
          variant='outline'
          disabled={block !== null || isOpening}
          title={block ?? undefined}
          onClick={onAdd}
        >
          {isOpening ? <Spinner className='size-3.5' /> : <CreditCard className='h-4 w-4' />}
          Add a payment method
        </Button>
      </div>
      {card ? (
        <BankCard card={card} />
      ) : (
        <BankCardPlaceholder
          message={status === 'missing' || !status ? NO_PAYMENT_METHOD : NO_CARD_DETAILS}
        />
      )}
      {block && <p className='text-xs text-muted-foreground'>{block}</p>}
      {refusal && (
        <p role='alert' className='text-sm text-destructive'>
          {refusal}
        </p>
      )}
    </div>
  )
}
