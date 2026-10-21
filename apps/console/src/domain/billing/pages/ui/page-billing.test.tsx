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
})
