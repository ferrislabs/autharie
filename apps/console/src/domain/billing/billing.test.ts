import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { ApiRequestError } from '@/api/api.fetch'
import {
  BYOC_MOVE_UNAVAILABLE,
  DATABASE_MOVE_UNAVAILABLE,
  NEEDS_BILLING_DETAILS,
  NEEDS_BILLING_RIGHT,
  NEEDS_PAYMENT_METHOD,
  accountOf,
  brandLabel,
  formatExpiry,
  maskedNumber,
  fittingTiers,
  formatMinor,
  hostingOf,
  isPaid,
  paidActionBlock,
  payNowUrl,
  tierChangeBlock,
} from './billing'

const account = (complete: boolean): Schemas.BillingAccountResponse => ({
  name: 'Acme',
  email: 'billing@acme.test',
  address: { line1: '1 rue de la Paix', postal_code: '75002', city: 'Paris', country: 'FR' },
  payment_method: complete ? 'valid' : 'missing',
  complete,
})

const tier = (
  code: string,
  engines: Schemas.DeploymentKind[],
  hostings: Schemas.Hosting[],
): Schemas.TierResponse => ({
  code,
  name: code,
  currency: 'EUR',
  engines,
  features: [],
  hostings,
  monthly_price: 9900,
  offer: 'standard',
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

  it('leave the decision to the server when the account is unknown', () => {
    expect(paidActionBlock({ canManage: true, account: undefined })).toBeNull()
  })

  it('still need the right when the account is unknown', () => {
    expect(paidActionBlock({ canManage: false, account: undefined })).toBe(NEEDS_BILLING_RIGHT)
  })
})

describe('the account a caller knows', () => {
  it('is unknown when the caller cannot view billing', () => {
    expect(accountOf(false, { data: account(true), error: null })).toBeUndefined()
  })

  it('is the data when there is some', () => {
    const data = account(true)
    expect(accountOf(true, { data, error: null })).toBe(data)
  })

  it('is none on a 404, which only says there is no account yet', () => {
    expect(accountOf(true, { data: undefined, error: new ApiRequestError(404, 'no account') })).toBeNull()
  })

  it('is unknown while loading or after any other failure', () => {
    expect(accountOf(true, { data: undefined, error: null })).toBeUndefined()
    expect(accountOf(true, { data: undefined, error: new ApiRequestError(500, 'broke') })).toBeUndefined()
    expect(accountOf(true, { data: undefined, error: new Error('network') })).toBeUndefined()
  })
})

describe('the tiers that fit', () => {
  const tiers = [
    tier('shared-ferriskey', ['ferriskey'], ['shared']),
    tier('cloud-ferriskey', ['ferriskey'], ['customer_cloud']),
    tier('shared-keycloak', ['keycloak'], ['shared']),
    tier('wide', ['ferriskey', 'keycloak'], ['shared', 'self_hosted', 'customer_cloud']),
  ]

  it('hold the engine and the hosting', () => {
    expect(
      fittingTiers(tiers, { engine: 'ferriskey', hosting: 'shared' }).map((t) => t.code),
    ).toEqual(['shared-ferriskey', 'wide'])
    expect(
      fittingTiers(tiers, { engine: 'ferriskey', hosting: 'customer_cloud' }).map((t) => t.code),
    ).toEqual(['cloud-ferriskey', 'wide'])
    expect(
      fittingTiers(tiers, { engine: 'keycloak', hosting: 'shared' }).map((t) => t.code),
    ).toEqual(['shared-keycloak', 'wide'])
  })

  it('are none when nothing holds both', () => {
    expect(fittingTiers(tiers, { engine: 'keycloak', hosting: 'self_hosted' }).map((t) => t.code)).toEqual([
      'wide',
    ])
    expect(fittingTiers([tiers[0]], { engine: 'keycloak', hosting: 'shared' })).toEqual([])
  })
})

describe('the hosting of a deployment', () => {
  it('follows its distribution', () => {
    expect(hostingOf('shared')).toBe('shared')
    expect(hostingOf('self_hosted')).toBe('self_hosted')
    expect(
      hostingOf({
        customer_cloud: {
          credential_id: 'c1',
          profile: {} as Schemas.ClusterProfile,
        },
      }),
    ).toBe('customer_cloud')
  })
})

describe('tier change block', () => {
  it('blocks a move between the free offer and a paid one, both ways', () => {
    expect(tierChangeBlock('free', { offer: 'standard' })).toBe(DATABASE_MOVE_UNAVAILABLE)
    expect(tierChangeBlock('free', { offer: 'scale' })).toBe(DATABASE_MOVE_UNAVAILABLE)
    expect(tierChangeBlock('standard', { offer: 'free' })).toBe(DATABASE_MOVE_UNAVAILABLE)
    expect(tierChangeBlock('scale', { offer: 'free' })).toBe(DATABASE_MOVE_UNAVAILABLE)
  })

  it('blocks a move to or from BYOC', () => {
    expect(tierChangeBlock('private', { offer: 'standard' })).toBe(BYOC_MOVE_UNAVAILABLE)
    expect(tierChangeBlock('standard', { offer: 'private' })).toBe(BYOC_MOVE_UNAVAILABLE)
  })

  it('allows a move between paid offers of the same kind', () => {
    expect(tierChangeBlock('standard', { offer: 'scale' })).toBeNull()
    expect(tierChangeBlock('scale', { offer: 'standard' })).toBeNull()
    expect(tierChangeBlock('free', { offer: 'free' })).toBeNull()
    expect(tierChangeBlock('private', { offer: 'private' })).toBeNull()
  })

  it('blocks nothing while the current offer is unknown', () => {
    expect(tierChangeBlock(undefined, { offer: 'free' })).toBeNull()
    expect(tierChangeBlock(undefined, { offer: 'private' })).toBeNull()
    expect(tierChangeBlock(undefined, { offer: 'standard' })).toBeNull()
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
})

describe('a card as the customer recognises it', () => {
  const card = (brand: string, exp_month = 4, exp_year = 2027): Schemas.CardResponse => ({
    brand,
    last4: '4242',
    exp_month,
    exp_year,
  })

  it('writes the brands the way their owners do', () => {
    expect(brandLabel('visa')).toBe('Visa')
    expect(brandLabel('mastercard')).toBe('Mastercard')
    expect(brandLabel('amex')).toBe('American Express')
    expect(brandLabel('cartes_bancaires')).toBe('Cartes Bancaires')
  })

  it('still says something for a brand it does not know', () => {
    expect(brandLabel('some_new_network')).toBe('Some new network')
    expect(brandLabel('unknown')).toBe('Card')
  })

  it('pads the month and keeps two digits of the year', () => {
    expect(formatExpiry(card('visa'))).toBe('04/27')
    expect(formatExpiry(card('visa', 12, 2031))).toBe('12/31')
  })

  it('masks all but the last four digits, in four groups', () => {
    expect(maskedNumber(card('visa'))).toBe('**** **** **** 4242')
    expect(maskedNumber(card('some_new_network'))).toBe('**** **** **** 4242')
  })

  it('groups the digits of a 15-digit American Express card as 4, 6 and 5', () => {
    expect(maskedNumber(card('amex'))).toBe('**** ****** *4242')
  })

  it('groups the digits of a 14-digit Diners Club card as 4, 6 and 4', () => {
    expect(maskedNumber(card('diners'))).toBe('**** ****** 4242')
  })

  it('never reveals a digit other than the last four', () => {
    for (const brand of ['visa', 'amex', 'diners', 'unknown']) {
      expect(maskedNumber(card(brand)).replace(/[^0-9]/g, '')).toBe('4242')
    }
  })
})
