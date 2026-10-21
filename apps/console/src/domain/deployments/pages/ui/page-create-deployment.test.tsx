import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { NEEDS_PAYMENT_METHOD } from '@/domain/billing/billing'
import PageCreateDeployment, { CreateActions } from './page-create-deployment'

vi.mock('@tanstack/react-router', () => ({ useNavigate: () => () => {} }))
vi.mock('@/domain/organisations/hooks/use-organisation-path', () => ({
  useOrganisationPath: () => (path = '') => path,
}))

const noop = () => {}

function tier(overrides: Partial<Schemas.TierResponse> = {}): Schemas.TierResponse {
  return {
    code: 'pro',
    name: 'Pro',
    monthly_price: 2900,
    currency: 'eur',
    offer: 'standard',
    features: [],
    engines: ['ferriskey'],
    hostings: ['shared'],
    first_invoice: { period: '2026-10', amount: 3345 },
    ...overrides,
  }
}

function page(tiers: Schemas.TierResponse[]) {
  return renderToStaticMarkup(
    <PageCreateDeployment
      onSubmit={noop}
      offers={[]}
      offersLoading={false}
      releases={[]}
      releasesLoading={false}
      tiers={tiers}
      billing={{ canManage: true, account: null }}
    />,
  )
}

describe('the page without a tier on offer', () => {
  it('keeps the indicative note and shows no tier picker', () => {
    const html = page([])
    expect(html).toContain('Indicative price. Billing for deployments is being set up.')
    expect(html).not.toContain('aria-label="Tier"')
    expect(html).not.toContain('First invoice')
  })
})

describe('the page with tiers on offer', () => {
  it('shows the tier picker and the first invoice of the API, without the indicative note', () => {
    const html = page([tier()])
    expect(html).toContain('aria-label="Tier"')
    expect(html).toContain('€33.45')
    expect(html).not.toContain('Indicative price')
  })

  it('says why a paid tier cannot be created yet', () => {
    const html = page([tier()])
    expect(html).toContain('Add the billing details first')
  })

  it('does not block a tier that costs nothing', () => {
    const html = page([tier({ monthly_price: 0, first_invoice: null })])
    expect(html).not.toContain('Add the billing details first')
  })
})

describe('the create action', () => {
  it('is disabled with the reason when the billing blocks it, however ready the form is', () => {
    const html = renderToStaticMarkup(
      <CreateActions
        canSubmit
        block={NEEDS_PAYMENT_METHOD}
        isSubmitting={false}
        onCancel={noop}
      />,
    )
    expect(html).toContain(NEEDS_PAYMENT_METHOD)
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Create deployment<\/button>/)
  })

  it('is enabled without a reason when nothing blocks it', () => {
    const html = renderToStaticMarkup(
      <CreateActions canSubmit block={null} isSubmitting={false} onCancel={noop} />,
    )
    expect(html).not.toContain('role="status"')
    expect(html).not.toMatch(/<button[^>]*disabled=""[^>]*>Create deployment<\/button>/)
  })

  it('stays disabled when the form is not ready', () => {
    const html = renderToStaticMarkup(
      <CreateActions canSubmit={false} block={null} isSubmitting={false} onCancel={noop} />,
    )
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Create deployment<\/button>/)
  })
})
