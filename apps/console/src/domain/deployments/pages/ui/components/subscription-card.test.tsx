import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  BYOC_MOVE_UNAVAILABLE,
  DATABASE_MOVE_UNAVAILABLE,
  NEEDS_BILLING_RIGHT,
  NEEDS_PAYMENT_METHOD,
} from '@/domain/billing/billing'
import { SubscriptionCard, type SubscriptionCardProps } from './subscription-card'

const noop = () => {}

const subscription = (
  monthly_price: number,
  tier = 'standard-2026',
  offer: Schemas.Offer = 'standard',
): Schemas.SubscriptionResponse => ({
  tier,
  monthly_price,
  offer,
  features: [],
  starts_at: '2026-03-04T12:00:00Z',
})

const tier = (
  code: string,
  monthly_price: number,
  fit: Partial<Pick<Schemas.TierResponse, 'engines' | 'hostings' | 'offer'>> = {},
): Schemas.TierResponse => ({
  code,
  name: code.toUpperCase(),
  currency: 'EUR',
  engines: ['ferriskey'],
  features: [],
  hostings: ['shared'],
  monthly_price,
  offer: monthly_price > 0 ? 'standard' : 'free',
  ...fit,
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
  fit: { engine: 'ferriskey', hosting: 'shared' },
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

  it('is not shown when the catalogue is empty, and says it is not open yet', () => {
    const html = render({ tiers: [] })
    expect(html).not.toContain('Change tier')
    expect(html).not.toContain('<button')
    expect(html).toContain('The tier catalogue is not open yet.')
    expect(html).not.toContain('No other tier available.')
  })

  it('is not shown when the catalogue only holds the current tier, and says no other is available', () => {
    const html = render({ tiers: [tier('standard-2026', 9900)] })
    expect(html).not.toContain('Change tier')
    expect(html).not.toContain('<button')
    expect(html).toContain('No other tier available.')
    expect(html).not.toContain('The tier catalogue is not open yet.')
  })

  it('offers only the tiers that fit the engine and the hosting of the deployment', () => {
    const html = render({
      tiers: [
        tier('scale-2026', 34900),
        tier('keycloak-2026', 19900, { engines: ['keycloak'] }),
        tier('cloud-2026', 49900, { hostings: ['customer_cloud'] }),
        tier('wide-2026', 59900, { engines: ['ferriskey', 'keycloak'], hostings: ['shared', 'self_hosted'] }),
      ],
    })
    expect(html).toContain('SCALE-2026')
    expect(html).toContain('WIDE-2026')
    expect(html).not.toContain('KEYCLOAK-2026')
    expect(html).not.toContain('CLOUD-2026')
  })

  it('follows the hosting of the deployment', () => {
    const html = render({
      fit: { engine: 'ferriskey', hosting: 'customer_cloud' },
      tiers: [tier('scale-2026', 34900), tier('cloud-2026', 49900, { hostings: ['customer_cloud'] })],
    })
    expect(html).toContain('CLOUD-2026')
    expect(html).not.toContain('SCALE-2026')
  })

  it('says no other is available when nothing fits, although the catalogue is not empty', () => {
    const html = render({ tiers: [tier('keycloak-2026', 19900, { engines: ['keycloak'] })] })
    expect(html).toContain('No other tier available.')
    expect(html).not.toContain('The tier catalogue is not open yet.')
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

  it('leaves a paid tier to the server when the account is unknown', () => {
    const html = render({ access: { canManage: true, account: undefined } })
    expect(html).not.toContain('disabled=""')
    expect(html).not.toContain('Add the billing details first')
  })

  it('is disabled with the reason for a paid tier when there is no account', () => {
    const html = render({ access: { canManage: true, account: null } })
    expect(html).toContain('disabled=""')
    expect(html).toContain('Add the billing details first')
  })

  it('stays enabled for the free tier when the account is incomplete', () => {
    const html = render({
      subscription: subscription(0, 'sandbox-2026', 'free'),
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

  it('disables a tier that would move the database, with the reason', () => {
    const html = render({
      subscription: subscription(0, 'free-2026', 'free'),
      tiers: [tier('scale-2026', 34900)],
    })
    expect(html).toContain('disabled=""')
    expect(html).toContain(DATABASE_MOVE_UNAVAILABLE)
    expect(html).toContain('aria-describedby="tier-scale-2026-block"')
  })

  it('disables the free tier for a paid subscription, with the reason', () => {
    const html = render({ tiers: [tier('free-2026', 0)] })
    expect(html).toContain('disabled=""')
    expect(html).toContain(DATABASE_MOVE_UNAVAILABLE)
  })

  it('disables a BYOC tier for a subscription that is not BYOC, with the reason', () => {
    const html = render({
      tiers: [tier('byoc-2026', 49900, { offer: 'private', hostings: ['shared'] })],
    })
    expect(html).toContain('disabled=""')
    expect(html).toContain(BYOC_MOVE_UNAVAILABLE)
    expect(html).not.toContain(DATABASE_MOVE_UNAVAILABLE)
  })

  it('disables a tier that is not BYOC for a BYOC subscription, with the reason', () => {
    const html = render({
      subscription: subscription(49900, 'byoc-2026', 'private'),
      tiers: [tier('scale-2026', 34900)],
    })
    expect(html).toContain('disabled=""')
    expect(html).toContain(BYOC_MOVE_UNAVAILABLE)
  })

  it('keeps a tier of another size within the same offer enabled', () => {
    const html = render({ tiers: [tier('scale-2026', 34900, { offer: 'scale' })] })
    expect(html).not.toContain('disabled=""')
    expect(html).not.toContain(DATABASE_MOVE_UNAVAILABLE)
    expect(html).not.toContain(BYOC_MOVE_UNAVAILABLE)
  })

  it('shows the move reason over the billing reason when the move can never succeed', () => {
    const html = render({
      subscription: subscription(0, 'free-2026', 'free'),
      tiers: [tier('scale-2026', 34900)],
      access: { canManage: true, account: account(false) },
    })
    expect(html).toContain(DATABASE_MOVE_UNAVAILABLE)
    expect(html).not.toContain(NEEDS_PAYMENT_METHOD)
  })

  it('keeps the billing reason for a move that is allowed', () => {
    const html = render({ access: { canManage: true, account: account(false) } })
    expect(html).toContain(NEEDS_PAYMENT_METHOD)
    expect(html).not.toContain(DATABASE_MOVE_UNAVAILABLE)
    expect(html).not.toContain(BYOC_MOVE_UNAVAILABLE)
  })

  it('tells that a tier of another size resizes the deployment and restarts its database once', () => {
    const html = render()
    expect(html).toContain(
      'Switching to a tier of another size resizes the deployment. Its database restarts once.',
    )
  })

  it('does not tell it without the right to change tiers', () => {
    const html = render({ access: { canManage: false, account: account(true) } })
    expect(html).not.toContain('Its database restarts once.')
  })

  it('does not tell it when there is no tier to switch to', () => {
    const html = render({ tiers: [] })
    expect(html).not.toContain('Its database restarts once.')
  })
})
