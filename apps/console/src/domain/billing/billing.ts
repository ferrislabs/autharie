import type { Schemas } from '@/api/api.client'

const EUR = new Intl.NumberFormat('en-GB', { style: 'currency', currency: 'EUR' })

const UNPAID: ReadonlyArray<Schemas.StatementViewStatus> = ['sent', 'payment_failed', 'uncollectible']

export const NEEDS_BILLING_RIGHT = 'Changing a paid subscription needs the Manage billing permission.'
export const NEEDS_BILLING_DETAILS = 'Add the billing details first: name, e-mail and address.'
export const NEEDS_PAYMENT_METHOD = 'Add a valid payment method first.'

export type BillingAccess = {
  canManage: boolean
  account: Schemas.BillingAccountResponse | null
}

export function formatMinor(minor: number): string {
  return EUR.format(minor / 100)
}

export function isPaid(tier: { monthly_price: number }): boolean {
  return tier.monthly_price > 0
}

export function paidActionBlock(access: BillingAccess): string | null {
  if (!access.canManage) return NEEDS_BILLING_RIGHT
  if (!access.account) return NEEDS_BILLING_DETAILS
  if (!access.account.complete) return NEEDS_PAYMENT_METHOD
  return null
}

export function payNowUrl(statement: {
  status: Schemas.StatementViewStatus
  hosted_invoice_url?: string | null
}): string | null {
  if (!UNPAID.includes(statement.status)) return null
  return statement.hosted_invoice_url ?? null
}

export function currentPeriod(now: Date): string {
  const month = String(now.getUTCMonth() + 1).padStart(2, '0')
  return `${now.getUTCFullYear()}-${month}`
}
