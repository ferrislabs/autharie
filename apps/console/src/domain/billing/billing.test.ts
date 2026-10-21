import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  NEEDS_BILLING_DETAILS,
  NEEDS_BILLING_RIGHT,
  NEEDS_PAYMENT_METHOD,
  currentPeriod,
  formatMinor,
  isPaid,
  paidActionBlock,
  payNowUrl,
} from './billing'

const account = (complete: boolean): Schemas.BillingAccountResponse => ({
  name: 'Acme',
  email: 'billing@acme.test',
  address: { line1: '1 rue de la Paix', postal_code: '75002', city: 'Paris', country: 'FR' },
  payment_method: complete ? 'valid' : 'missing',
  complete,
})

describe('paid actions', () => {
  it('are allowed with the right and a complete account', () => {
    expect(paidActionBlock({ canManage: true, account: account(true) })).toBeNull()
  })

  it('say the permission is missing before anything else', () => {
    expect(paidActionBlock({ canManage: false, account: account(true) })).toBe(NEEDS_BILLING_RIGHT)
  })

  it('say the billing details are missing when there is no account', () => {
    expect(paidActionBlock({ canManage: true, account: null })).toBe(NEEDS_BILLING_DETAILS)
  })

  it('say a payment method is missing when the account is incomplete', () => {
    expect(paidActionBlock({ canManage: true, account: account(false) })).toBe(NEEDS_PAYMENT_METHOD)
  })
})

describe('pay now', () => {
  it('links an unpaid statement to its hosted invoice', () => {
    for (const status of ['sent', 'payment_failed', 'uncollectible'] as const) {
      expect(payNowUrl({ status, hosted_invoice_url: 'https://invoice.stripe.test/i/1' })).toBe(
        'https://invoice.stripe.test/i/1'
      )
    }
  })

  it('offers nothing for a paid statement or an estimate', () => {
    expect(payNowUrl({ status: 'paid', hosted_invoice_url: 'https://invoice.stripe.test/i/1' })).toBeNull()
    expect(payNowUrl({ status: 'estimate', hosted_invoice_url: null })).toBeNull()
  })
})

describe('amounts and periods', () => {
  it('formats minor units as euros', () => {
    expect(formatMinor(4900)).toBe('€49.00')
    expect(formatMinor(3345)).toBe('€33.45')
  })

  it('treats a zero price as free', () => {
    expect(isPaid({ monthly_price: 0 })).toBe(false)
    expect(isPaid({ monthly_price: 1 })).toBe(true)
  })

  it('names the current period in UTC', () => {
    expect(currentPeriod(new Date('2026-11-30T23:30:00-02:00'))).toBe('2026-12')
    expect(currentPeriod(new Date('2026-01-05T00:00:00Z'))).toBe('2026-01')
  })
})
