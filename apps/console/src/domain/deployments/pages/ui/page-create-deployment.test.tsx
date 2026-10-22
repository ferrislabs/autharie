import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { NEEDS_PAYMENT_METHOD, type BillingAccess } from '@/domain/billing/billing'
import type { OfferAvailability } from '../../offers'
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

function entry(offer: Schemas.Offer, open: boolean): OfferAvailability {
  return {
    offer,
    open,
    resources: { cpu_millis: 500, memory_mib: 1024, storage_gib: 1 },
    shares_a_cluster: true,
  }
}

function page(
  tiers: Schemas.TierResponse[],
  over: {
    offers?: OfferAvailability[]
    billing?: BillingAccess
    tiersLoading?: boolean
  } = {},
) {
  return renderToStaticMarkup(
    <PageCreateDeployment
      onSubmit={noop}
      offers={over.offers ?? []}
      offersLoading={false}
      releases={[]}
      releasesLoading={false}
      tiers={tiers}
      tiersLoading={over.tiersLoading ?? false}
      billing={over.billing ?? { canManage: true, account: null }}
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

  it('leaves a paid tier to the server when the account is unknown', () => {
    const html = page([tier()], { billing: { canManage: true, account: undefined } })
    expect(html).not.toContain('Add the billing details first')
  })
})

describe('the tiers that fit', () => {
  it('leave out a tier for another engine or another hosting', () => {
    const html = page([
      tier({ code: 'keycloak', name: 'Keycloak only', engines: ['keycloak'] }),
      tier({ code: 'cloud', name: 'Cloud only', hostings: ['customer_cloud'] }),
      tier({ code: 'fits', name: 'Fits' }),
    ])
    expect(html).toContain('Fits')
    expect(html).not.toContain('Keycloak only')
    expect(html).not.toContain('Cloud only')
  })

  it('show no tier picker and keep the grid when no tier fits', () => {
    const html = page([tier({ hostings: ['customer_cloud'] })])
    expect(html).not.toContain('aria-label="Tier"')
    expect(html).toContain('Expected volume')
    expect(html).toContain('Indicative price. Billing for deployments is being set up.')
  })
})

describe('the plan and the volume', () => {
  it('are asked for when no tier is on offer', () => {
    const html = page([])
    expect(html).toContain('Expected volume')
    expect(html).toContain('Starter')
  })

  it('are hidden once a tier is selected, since the tier brings its offer', () => {
    const html = page([tier()])
    expect(html).not.toContain('Expected volume')
    expect(html).not.toContain('Starter')
    expect(html).not.toContain('Business')
  })

  it('and the free offer card are hidden for a tier when the plan only opens the free offer', () => {
    const offers = [entry('free', true)]
    expect(page([], { offers })).toContain('A FerrisKey of your own')
    expect(page([tier({ monthly_price: 0, first_invoice: null })], { offers })).not.toContain(
      'A FerrisKey of your own',
    )
  })

  it('do not warn about the offer they derive when a tier is selected', () => {
    const offers = [entry('standard', false), entry('scale', true)]
    expect(page([], { offers })).toContain('fits this choice, but it is not available on your plan')
    expect(page([tier()], { offers })).not.toContain(
      'fits this choice, but it is not available on your plan',
    )
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

describe('the layout', () => {
  const variants = {
    'with plans and volume': () => page([]),
    'with a tier': () => page([tier()]),
    'on the free offer only': () => page([], { offers: [entry('free', true)] }),
  }

  it.each(Object.entries(variants))('puts the summary and the actions beside the form %s', (_, render) => {
    const html = render()
    expect(html).toContain('lg:grid-cols-[minmax(0,1fr)_18rem]')
    expect(html).not.toContain('max-w-3xl')
    const aside = html.slice(html.indexOf('<aside'), html.indexOf('</aside>'))
    expect(aside).toContain('Monthly price')
    expect(aside).toContain('Create deployment')
    expect(aside).toContain('Cancel')
    expect(html.indexOf('Details')).toBeLessThan(html.indexOf('<aside'))
  })

  it('asks for the details, the volume, then the plan', () => {
    const html = page([])
    const at = (title: string) => html.indexOf(`<h2 class="text-base font-semibold">${title}</h2>`)
    expect(at('Details')).toBeGreaterThanOrEqual(0)
    expect(at('Details')).toBeLessThan(at('Expected volume'))
    expect(at('Expected volume')).toBeLessThan(at('Plan'))
  })
})

describe('the details', () => {
  it('suggests a name', () => {
    expect(page([])).toContain('placeholder="My deployment"')
  })

  it('offers each environment as a card with a description, not a select', () => {
    const html = page([])
    expect(html).not.toContain('role="combobox"')
    for (const label of ['Development', 'Staging', 'Production']) {
      expect(html).toContain(label)
    }
    expect(html).toContain('To build and test an integration.')
    expect(html).toContain('To check a change before it reaches production.')
    expect(html).toContain('For the users of your product.')
  })
})
