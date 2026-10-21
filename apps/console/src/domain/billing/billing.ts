import type { Schemas } from '@/api/api.client'
import { ApiRequestError } from '@/api/api.fetch'

const EUR = new Intl.NumberFormat('en-GB', { style: 'currency', currency: 'EUR' })

const UNPAID: ReadonlyArray<Schemas.StatementViewStatus> = ['sent', 'payment_failed', 'uncollectible']

export const NEEDS_BILLING_RIGHT = 'Changing a paid subscription needs the Manage billing permission.'
export const NEEDS_BILLING_DETAILS = 'Add the billing details first: name, e-mail and address.'
export const NEEDS_PAYMENT_METHOD = 'Add a valid payment method first.'

export const DATABASE_MOVE_UNAVAILABLE =
  'Moving between the free tier and a paid tier is not available yet. Create a deployment on the tier you want and move your data to it.'
export const BYOC_MOVE_UNAVAILABLE = 'Moving to or from BYOC is not available yet.'

export type BillingAccess = {
  canManage: boolean
  account: Schemas.BillingAccountResponse | null | undefined
}

export type TierFit = {
  engine: Schemas.DeploymentKind
  hosting: Schemas.Hosting
}

const CARD_BRANDS: ReadonlyMap<string, string> = new Map([
  ['visa', 'Visa'],
  ['mastercard', 'Mastercard'],
  ['amex', 'American Express'],
  ['diners', 'Diners Club'],
  ['discover', 'Discover'],
  ['jcb', 'JCB'],
  ['unionpay', 'UnionPay'],
  ['cartes_bancaires', 'Cartes Bancaires'],
  ['unknown', 'Card'],
])

function brandLabel(brand: string): string {
  const known = CARD_BRANDS.get(brand)
  if (known) return known
  const words = brand.replace(/_/g, ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

export function describeCard(card: Schemas.CardResponse): string {
  const month = String(card.exp_month).padStart(2, '0')
  const year = String(card.exp_year % 100).padStart(2, '0')
  return `${brandLabel(card.brand)} •••• ${card.last4} · exp. ${month}/${year}`
}

export function formatMinor(minor: number): string {
  return EUR.format(minor / 100)
}

export function isPaid(tier: { monthly_price: number }): boolean {
  return tier.monthly_price > 0
}

export function paidActionBlock(access: BillingAccess): string | null {
  if (!access.canManage) return NEEDS_BILLING_RIGHT
  if (access.account === undefined) return null
  if (access.account === null) return NEEDS_BILLING_DETAILS
  if (!access.account.complete) return NEEDS_PAYMENT_METHOD
  return null
}

export function tierChangeBlock(
  currentOffer: Schemas.Offer | undefined,
  tier: { offer: Schemas.Offer },
): string | null {
  if (currentOffer === undefined) return null
  if ((currentOffer === 'free') !== (tier.offer === 'free')) return DATABASE_MOVE_UNAVAILABLE
  if ((currentOffer === 'private') !== (tier.offer === 'private')) return BYOC_MOVE_UNAVAILABLE
  return null
}

export function payNowUrl(statement: {
  status: Schemas.StatementViewStatus
  hosted_invoice_url?: string | null
}): string | null {
  if (!UNPAID.includes(statement.status)) return null
  return statement.hosted_invoice_url ?? null
}

export function accountOf(
  canView: boolean,
  query: { data?: Schemas.BillingAccountResponse; error: Error | null },
): Schemas.BillingAccountResponse | null | undefined {
  if (!canView) return undefined
  if (query.data) return query.data
  if (query.error instanceof ApiRequestError && query.error.status === 404) return null
  return undefined
}

export function fittingTiers(
  tiers: ReadonlyArray<Schemas.TierResponse>,
  fit: TierFit,
): Schemas.TierResponse[] {
  return tiers.filter(
    (tier) => tier.engines.includes(fit.engine) && tier.hostings.includes(fit.hosting),
  )
}

export function hostingOf(distribution: Schemas.Distribution): Schemas.Hosting {
  if (distribution === 'shared') return 'shared'
  if (distribution === 'self_hosted') return 'self_hosted'
  return 'customer_cloud'
}
