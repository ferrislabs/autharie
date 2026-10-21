import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { NEEDS_BILLING_RIGHT, NEEDS_PAYMENT_METHOD } from '@/domain/billing/billing'
import { SubscriptionCard, type SubscriptionCardProps } from './subscription-card'

const noop = () => {}

const subscription = (monthly_price: number, tier = 'standard-2026'): Schemas.SubscriptionResponse => ({
  tier,
  monthly_price,
  offer: 'standard',
  features: [],
  starts_at: '2026-03-04T12:00:00Z',
})

const tier = (code: string, monthly_price: number): Schemas.TierResponse => ({
  code,
  name: code.toUpperCase(),
  currency: 'EUR',
  engines: ['ferriskey'],
  features: [],
  hostings: ['shared'],
  monthly_price,
  offer: monthly_price > 0 ? 'standard' : 'free',
})

const account = (complete: boolean): Schemas.BillingAccountResponse => ({
  name: 'Acme',
  email: 'billing@acme.test',
  address: { line1: '1 rue de la Paix', postal_code: '75002', city: 'Paris', country: 'FR' },
  payment_method: complete ? 'valid' : 'missing',
  complete,
})

const props = (over: Partial<SubscriptionCardProps> = {}): SubscriptionCardProps => ({
  subscription: subscription(9900),
  isLoading: false,
  isError: false,
  tiers: [tier('scale-2026', 34900)],
  access: { canManage: true, account: account(true) },
  changing: null,
  onChange: noop,
  ...over,
})

const render = (over: Partial<SubscriptionCardProps> = {}) =>
  renderToStaticMarkup(<SubscriptionCard {...props(over)} />)

describe('the subscription card', () => {
  it('shows the current tier, its price and since when', () => {
    const html = render()
    expect(html).toContain('Subscription')
    expect(html).toContain('standard-2026')
    expect(html).toContain('€99.00 per month')
    expect(html).toContain('Since 4 Mar 2026')
  })

  it('says Free for a zero price', () => {
    const html = render({ subscription: subscription(0, 'free-2026'), tiers: [] })
    expect(html).toContain('free-2026')
    expect(html).toContain('Free')
    expect(html).not.toContain('per month')
  })

  it('shows a skeleton while loading', () => {
    const html = render({ subscription: undefined, isLoading: true })
    expect(html).not.toContain('Since')
  })

  it('says when the subscription could not be read', () => {
    const html = render({ subscription: undefined, isError: true })
    expect(html).toContain('The subscription could not be loaded.')
  })
})

describe('the tier change', () => {
  it('lists the selectable tiers other than the current one', () => {
    const html = render({
      subscription: subscription(9900, 'standard-2026'),
      tiers: [tier('standard-2026', 9900), tier('scale-2026', 34900), tier('free-2026', 0)],
    })
    expect(html).toContain('Change tier')
    expect(html).toContain('SCALE-2026')
    expect(html).toContain('€349.00 per month')
    expect(html).toContain('FREE-2026')
    expect(html).not.toContain('STANDARD-2026')
  })

  it('is not shown when no tier is selectable, and says the catalogue is not open yet', () => {
    const html = render({ tiers: [] })
    expect(html).not.toContain('Change tier')
    expect(html).not.toContain('<button')
    expect(html).toContain('The tier catalogue is not open yet.')
  })

  it('is not shown while the tiers are unknown', () => {
    const html = render({ tiers: undefined })
    expect(html).not.toContain('Change tier')
    expect(html).not.toContain('The tier catalogue is not open yet.')
  })

  it('is enabled for a paid tier with the right and a complete account', () => {
    const html = render()
    expect(html).toContain('<button')
    expect(html).not.toContain('disabled=""')
    expect(html).not.toContain(NEEDS_PAYMENT_METHOD)
  })

  it('is disabled with the reason for a paid tier when the account is incomplete', () => {
    const html = render({ access: { canManage: true, account: account(false) } })
    expect(html).toContain('disabled=""')
    expect(html).toContain(NEEDS_PAYMENT_METHOD)
  })

  it('is disabled with the reason for a paid tier when there is no account', () => {
    const html = render({ access: { canManage: true, account: null } })
    expect(html).toContain('disabled=""')
    expect(html).toContain('Add the billing details first')
  })

  it('stays enabled for the free tier when the account is incomplete', () => {
    const html = render({
      tiers: [tier('free-2026', 0)],
      access: { canManage: true, account: account(false) },
    })
    expect(html).not.toContain('disabled=""')
    expect(html).not.toContain(NEEDS_PAYMENT_METHOD)
  })

  it('is disabled with NEEDS_BILLING_RIGHT without Manage billing, free tier included', () => {
    const html = render({
      tiers: [tier('free-2026', 0), tier('scale-2026', 34900)],
      access: { canManage: false, account: account(true) },
    })
    expect(html.match(/disabled=""/g)).toHaveLength(2)
    expect(html).toContain(NEEDS_BILLING_RIGHT)
  })

  it('disables every button while a change is in flight', () => {
    const html = render({
      tiers: [tier('free-2026', 0), tier('scale-2026', 34900)],
      changing: 'scale-2026',
    })
    expect(html.match(/disabled=""/g)).toHaveLength(2)
  })

  it('shows the error of the API in place', () => {
    const html = render({ error: 'The tier cannot be selected.' })
    expect(html).toContain('role="alert"')
    expect(html).toContain('The tier cannot be selected.')
  })
})
