import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  NEEDS_BILLING_DETAILS,
  NEEDS_BILLING_RIGHT,
  NEEDS_PAYMENT_METHOD,
} from '@/domain/billing/billing'
import type { CreateDeploymentForm } from './create-deployment-request'
import {
  apiHostingOf,
  canSubmit,
  createBlock,
  submission,
  toCreateDeploymentRequest,
  type SubmissionInput,
} from './create-deployment-request'

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
    first_invoice: null,
    ...overrides,
  }
}

function account(complete: boolean): Schemas.BillingAccountResponse {
  return {
    name: 'Acme',
    email: 'billing@acme.test',
    address: { line1: '1 rue de Rivoli', postal_code: '75001', city: 'Paris', country: 'FR' },
    complete,
    payment_method: complete ? 'valid' : 'missing',
  }
}

function form(overrides: Partial<CreateDeploymentForm> = {}): CreateDeploymentForm {
  return {
    name: 'acme api',
    version: '0.5.0',
    environment: 'production',
    offer: 'standard',
    ...overrides,
  }
}

describe('what the form sends', () => {
  /**
   * The six infrastructure decisions the request used to carry are gone. The
   * platform derives the namespace and the sizing from the offer and decides
   * where the thing runs, and refuses a request that still names any of them
   * rather than half-honouring it.
   */
  it('carries nothing the platform decides for itself', () => {
    expect(toCreateDeploymentRequest(form())).toEqual({
      name: 'acme api',
      kind: 'ferriskey',
      version: '0.5.0',
      environment: 'production',
      offer: 'standard',
    })
  })

  it('carries the offer the customer chose, whichever it is', () => {
    expect(toCreateDeploymentRequest(form({ offer: 'private' })).offer).toBe('private')
  })

  /**
   * A region is the fleet seen from outside: choosing one is choosing which
   * cluster serves you, which is an operator's decision. The API refuses a
   * request that names one, so sending it would turn every create into a 400.
   */
  it('names no region, because where a deployment runs is not the customer’s call', () => {
    expect(toCreateDeploymentRequest(form())).not.toHaveProperty('region')
  })

  it('sends no distribution, so the deployment stays on the shared platform', () => {
    expect(toCreateDeploymentRequest(form())).not.toHaveProperty('distribution')
  })

  it('is always FerrisKey', () => {
    expect(toCreateDeploymentRequest(form()).kind).toBe('ferriskey')
    expect(toCreateDeploymentRequest(form({ offer: 'free' })).kind).toBe('ferriskey')
  })
})

describe('the tier', () => {
  it('is sent when one is selected', () => {
    expect(toCreateDeploymentRequest(form({ tier: 'pro' })).tier).toBe('pro')
  })

  it('is left out when none is selected', () => {
    expect(toCreateDeploymentRequest(form())).not.toHaveProperty('tier')
    expect(toCreateDeploymentRequest(form({ tier: undefined }))).not.toHaveProperty('tier')
  })
})

describe('what blocks the creation', () => {
  const ready = { canManage: true, account: account(true) }

  it('is nothing when no tier is selected', () => {
    expect(createBlock(undefined, { canManage: false, account: null })).toBeNull()
  })

  it('is nothing for a tier that costs nothing', () => {
    expect(createBlock(tier({ monthly_price: 0 }), { canManage: false, account: null })).toBeNull()
  })

  it('is nothing for a paid tier when the billing is in order', () => {
    expect(createBlock(tier(), ready)).toBeNull()
  })

  it('says the right is missing for a paid tier without Manage billing', () => {
    expect(createBlock(tier(), { ...ready, canManage: false })).toBe(NEEDS_BILLING_RIGHT)
  })

  it('says the details are missing for a paid tier without an account', () => {
    expect(createBlock(tier(), { canManage: true, account: null })).toBe(NEEDS_BILLING_DETAILS)
  })

  it('says the payment method is missing for a paid tier with an incomplete account', () => {
    expect(createBlock(tier(), { canManage: true, account: account(false) })).toBe(
      NEEDS_PAYMENT_METHOD,
    )
  })

  it('leaves a paid tier to the server when the account is unknown', () => {
    expect(createBlock(tier(), { canManage: true, account: undefined })).toBeNull()
  })

  it('still needs the right for a paid tier when the account is unknown', () => {
    expect(createBlock(tier(), { canManage: false, account: undefined })).toBe(NEEDS_BILLING_RIGHT)
  })
})

describe('the hosting of the grid', () => {
  it('maps to the hosting the platform derives from the distribution', () => {
    expect(apiHostingOf('managed')).toBe('shared')
    expect(apiHostingOf('byoc')).toBe('customer_cloud')
  })
})

function input(overrides: Partial<SubmissionInput> = {}): SubmissionInput {
  return {
    name: 'acme api',
    version: '0.5.0',
    environment: 'production',
    tier: undefined,
    gridOffer: 'standard',
    loading: false,
    submitting: false,
    block: null,
    ...overrides,
  }
}

describe('what the page submits', () => {
  it('sends the offer of the tier with the tier, whatever the grid derives', () => {
    const sent = submission(input({ tier: tier({ code: 'scale-2026', offer: 'scale' }), gridOffer: 'sandbox' }))
    expect(sent).not.toBeNull()
    expect(toCreateDeploymentRequest(sent!)).toEqual({
      name: 'acme api',
      kind: 'ferriskey',
      version: '0.5.0',
      environment: 'production',
      offer: 'scale',
      tier: 'scale-2026',
    })
  })

  it('does not need the grid to derive an offer when a tier is selected', () => {
    const sent = submission(input({ tier: tier({ offer: 'standard' }), gridOffer: undefined }))
    expect(sent?.offer).toBe('standard')
    expect(canSubmit(input({ tier: tier(), gridOffer: undefined }))).toBe(true)
  })

  it('sends the offer of the grid and no tier when none is selected', () => {
    const sent = submission(input({ gridOffer: 'sandbox' }))
    expect(sent).toEqual({
      name: 'acme api',
      version: '0.5.0',
      environment: 'production',
      offer: 'sandbox',
    })
  })

  it('needs an offer from the grid when no tier is selected', () => {
    expect(canSubmit(input({ gridOffer: undefined }))).toBe(false)
    expect(submission(input({ gridOffer: undefined }))).toBeNull()
  })

  it('submits nothing while the tiers are loading', () => {
    expect(canSubmit(input({ loading: true }))).toBe(false)
    expect(submission(input({ loading: true }))).toBeNull()
  })

  it('submits nothing when the billing blocks the creation', () => {
    expect(submission(input({ tier: tier(), block: NEEDS_PAYMENT_METHOD }))).toBeNull()
  })

  it('submits nothing without a name, without a version or while submitting', () => {
    expect(submission(input({ name: '  ' }))).toBeNull()
    expect(submission(input({ version: '' }))).toBeNull()
    expect(submission(input({ submitting: true }))).toBeNull()
  })
})
