import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { NEEDS_BILLING_DETAILS, formatMinor } from '@/domain/billing/billing'
import { BillingAccountForm, NEEDS_MANAGE_TO_EDIT } from './billing-account-form'
import { MonthToDate } from './month-to-date'
import { NEEDS_MANAGE_TO_ADD, PaymentMethodPanel } from './payment-method-panel'
import { StatementsTable } from './statements-table'

const noop = () => {}

const account: Schemas.BillingAccountResponse = {
  name: 'Acme',
  email: 'billing@acme.test',
  address: { line1: '1 rue de la Paix', postal_code: '75002', city: 'Paris', country: 'FR' },
  vat_number: 'FR12345678901',
  payment_method: 'valid',
  complete: true,
}

const statement = (
  period: string,
  status: Schemas.StatementStatus,
  hosted_invoice_url?: string,
): Schemas.StatementSummary => ({ period, status, total: 12345, hosted_invoice_url })

const line = (deployment_id: string, amount: number): Schemas.StatementLineResponse => ({
  deployment_id,
  tier: 'business',
  monthly_price: 22500,
  starts_at: '2026-10-01T00:00:00Z',
  ends_at: '2026-10-21T00:00:00Z',
  covered_seconds: 1000,
  amount,
})

describe('the statements', () => {
  it('link an unpaid one to its hosted invoice', () => {
    const html = renderToStaticMarkup(
      <StatementsTable
        statements={[statement('2026-09', 'sent', 'https://pay.test/inv_1')]}
      />,
    )
    expect(html).toContain('Pay now')
    expect(html).toContain('href="https://pay.test/inv_1"')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener"')
  })

  it('offer no link on a paid one', () => {
    const html = renderToStaticMarkup(
      <StatementsTable
        statements={[statement('2026-08', 'paid', 'https://pay.test/inv_2')]}
      />,
    )
    expect(html).not.toContain('Pay now')
    expect(html).not.toContain('https://pay.test/inv_2')
  })

  it('offer no link on an unpaid one the provider gave no address for', () => {
    const html = renderToStaticMarkup(
      <StatementsTable statements={[statement('2026-09', 'payment_failed')]} />,
    )
    expect(html).not.toContain('Pay now')
  })

  it('show the period and the total of each', () => {
    const html = renderToStaticMarkup(
      <StatementsTable
        statements={[statement('2026-09', 'sent', 'https://pay.test/inv_1'), statement('2026-08', 'paid')]}
      />,
    )
    expect(html).toContain('2026-09')
    expect(html).toContain('2026-08')
    expect(html).toContain(formatMinor(12345))
  })

  it('say so when none was issued yet', () => {
    const html = renderToStaticMarkup(<StatementsTable statements={[]} />)
    expect(html).toContain('No statement yet')
  })
})

describe('the billing account form', () => {
  it('is disabled without Manage billing, and says why', () => {
    const html = renderToStaticMarkup(
      <BillingAccountForm account={account} canManage={false} isSaving={false} onSave={noop} />,
    )
    const inputs = html.match(/<input[^>]*>/g) ?? []
    const buttons = html.match(/<button[^>]*>/g) ?? []
    expect(inputs.length).toBe(9)
    expect(inputs.every((input) => input.includes('disabled=""'))).toBe(true)
    expect(buttons.length).toBe(1)
    expect(buttons.every((button) => button.includes('disabled=""'))).toBe(true)
    expect(html).toContain(NEEDS_MANAGE_TO_EDIT)
  })

  it('is editable with Manage billing, and starts from the account', () => {
    const html = renderToStaticMarkup(
      <BillingAccountForm account={account} canManage isSaving={false} onSave={noop} />,
    )
    const inputs = html.match(/<input[^>]*>/g) ?? []
    expect(inputs.some((input) => input.includes('disabled=""'))).toBe(false)
    expect(html).toContain('value="Acme"')
    expect(html).toContain('value="75002"')
    expect(html).not.toContain(NEEDS_MANAGE_TO_EDIT)
  })

  it('is empty when the organisation has no account yet', () => {
    const html = renderToStaticMarkup(
      <BillingAccountForm account={null} canManage isSaving={false} onSave={noop} />,
    )
    expect(html).not.toContain('value="Acme"')
  })

  it('shows what the platform refused', () => {
    const html = renderToStaticMarkup(
      <BillingAccountForm
        account={account}
        canManage
        isSaving={false}
        refusal='the VAT number is not valid'
        onSave={noop}
      />,
    )
    expect(html).toContain('the VAT number is not valid')
  })
})

describe('the payment method', () => {
  it('cannot be added without Manage billing, and says why', () => {
    const html = renderToStaticMarkup(
      <PaymentMethodPanel
        status='missing'
        hasAccount
        canManage={false}
        isOpening={false}
        onAdd={noop}
      />,
    )
    const button = (html.match(/<button[^>]*>/g) ?? [])[0]
    expect(button).toContain('disabled=""')
    expect(html).toContain(NEEDS_MANAGE_TO_ADD)
  })

  it('cannot be added before the billing details, and says why', () => {
    const html = renderToStaticMarkup(
      <PaymentMethodPanel hasAccount={false} canManage isOpening={false} onAdd={noop} />,
    )
    const button = (html.match(/<button[^>]*>/g) ?? [])[0]
    expect(button).toContain('disabled=""')
    expect(html).toContain(NEEDS_BILLING_DETAILS)
  })

  it('can be added by whoever manages billing', () => {
    const html = renderToStaticMarkup(
      <PaymentMethodPanel status='failing' hasAccount canManage isOpening={false} onAdd={noop} />,
    )
    const button = (html.match(/<button[^>]*>/g) ?? [])[0]
    expect(button).not.toContain('disabled=""')
    expect(html).toContain('Add a payment method')
    expect(html).toContain('Failing')
  })

  it('says which card is on file', () => {
    const html = renderToStaticMarkup(
      <PaymentMethodPanel
        status='valid'
        card={{ brand: 'visa', last4: '4242', exp_month: 4, exp_year: 2027 }}
        hasAccount
        canManage
        isOpening={false}
        onAdd={noop}
      />,
    )
    expect(html).toContain('Visa •••• 4242 · exp. 04/27')
  })

  it('says nothing about a card when none is known', () => {
    const html = renderToStaticMarkup(
      <PaymentMethodPanel status='valid' card={null} hasAccount canManage isOpening={false} onAdd={noop} />,
    )
    expect(html).not.toContain('••••')
  })

  it('shows what the platform refused', () => {
    const html = renderToStaticMarkup(
      <PaymentMethodPanel
        hasAccount
        canManage
        isOpening={false}
        refusal='Billing is not enabled on this installation.'
        onAdd={noop}
      />,
    )
    expect(html).toContain('Billing is not enabled on this installation.')
  })
})

describe('the month to date', () => {
  const estimate: Schemas.StatementResponse = {
    period: '2026-10',
    status: 'estimate',
    total: 4050,
    lines: [line('dep-a', 1000), line('dep-b', 2500), line('dep-a', 550)],
  }

  it('sums the lines of each deployment and shows the total', () => {
    const html = renderToStaticMarkup(
      <MonthToDate estimate={estimate} names={new Map([['dep-a', 'Alpha']])} />,
    )
    expect(html).toContain('Alpha')
    expect(html).toContain(formatMinor(1550))
    expect(html).toContain('Unknown deployment (dep-b)')
    expect(html).toContain(formatMinor(2500))
    expect(html).toContain(formatMinor(4050))
    expect(html).not.toContain(formatMinor(1000))
  })

  it('does not show the raw id of a deployment missing from the list', () => {
    const html = renderToStaticMarkup(
      <MonthToDate
        estimate={{ ...estimate, lines: [line('0b6e3a1c-9d4f-4c7a-8e21-5f3d2a9b7c10', 500)] }}
        names={new Map()}
      />,
    )
    expect(html).toContain('Unknown deployment (0b6e3a1c)')
    expect(html).not.toContain('0b6e3a1c-9d4f')
  })

  it('says it is an estimate up to now', () => {
    const html = renderToStaticMarkup(<MonthToDate estimate={estimate} names={new Map()} />)
    expect(html).toContain('Estimate up to now')
  })

  it('says so when nothing is billed yet', () => {
    const html = renderToStaticMarkup(<MonthToDate names={new Map()} />)
    expect(html).toContain('Nothing billed yet this month')
  })
})
