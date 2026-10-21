import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { PageBilling } from './page-billing'

const noop = () => {}

const account: Schemas.BillingAccountResponse = {
  name: 'Acme',
  email: 'billing@acme.test',
  address: { line1: '1 rue de la Paix', postal_code: '75002', city: 'Paris', country: 'FR' },
  payment_method: 'missing',
  complete: false,
}

const props = {
  account,
  statements: [],
  names: new Map<string, string>(),
  canManage: true,
  isLoading: false,
  estimateLoading: false,
  isSaving: false,
  isOpening: false,
  onSave: noop,
  onAddPaymentMethod: noop,
}

describe('the billing page', () => {
  it('shows the four blocks and that the account is incomplete', () => {
    const html = renderToStaticMarkup(<PageBilling {...props} />)
    expect(html).toContain('Billing account')
    expect(html).toContain('Incomplete')
    expect(html).toContain('Payment method')
    expect(html).toContain('This month so far')
    expect(html).toContain('Statements')
  })

  it('shows an empty form and no badge when there is no account', () => {
    const html = renderToStaticMarkup(<PageBilling {...props} account={null} />)
    expect(html).not.toContain('Incomplete')
    expect(html).not.toContain('value="Acme"')
  })

  it('shows the failure instead of a form that would overwrite what it could not read', () => {
    const html = renderToStaticMarkup(
      <PageBilling {...props} account={null} accountFailure='you may not view this' />,
    )
    expect(html).toContain('you may not view this')
    expect(html).not.toContain('<form')
  })

  it('shows only a skeleton during the first load', () => {
    const html = renderToStaticMarkup(<PageBilling {...props} isLoading />)
    expect(html).toContain('data-slot="skeleton"')
    expect(html).not.toContain('<form')
    expect(html).not.toContain('Billing account')
  })

  it('keeps the form while the month to date loads', () => {
    const html = renderToStaticMarkup(<PageBilling {...props} estimateLoading />)
    expect(html).toContain('<form')
    expect(html).toContain('value="Acme"')
    expect(html).toContain('data-slot="skeleton"')
    expect(html).not.toContain('Nothing billed yet this month')
  })

  it('keeps an empty form when the organisation has no account and the page refetches', () => {
    const html = renderToStaticMarkup(<PageBilling {...props} account={null} estimateLoading />)
    expect(html).toContain('<form')
    expect(html).not.toContain('value="Acme"')
  })

  it('says why the month to date cannot be shown instead of saying nothing was billed', () => {
    const html = renderToStaticMarkup(
      <PageBilling {...props} estimateFailure='no statement for this period' />,
    )
    expect(html).toContain('no statement for this period')
    expect(html).not.toContain('Nothing billed yet this month')
  })

  it('says nothing was billed when the estimate is there and empty', () => {
    const html = renderToStaticMarkup(
      <PageBilling
        {...props}
        estimate={{ period: '2026-10', status: 'estimate', total: 0, lines: [] }}
      />,
    )
    expect(html).toContain('Nothing billed yet this month')
  })
})
