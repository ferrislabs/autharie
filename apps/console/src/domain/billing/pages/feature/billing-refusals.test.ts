import { describe, expect, it } from 'vitest'
import { ApiRequestError } from '@/api/api.fetch'
import { BILLING_OFF, accountRefusal, loadFailure, paymentMethodRefusal } from './billing-refusals'

describe('the account refusal', () => {
  it('is the message of a 422', () => {
    expect(accountRefusal(new ApiRequestError(422, 'the country is not an ISO code'))).toBe(
      'the country is not an ISO code',
    )
  })

  it('is the message of any other failure', () => {
    expect(accountRefusal(new ApiRequestError(502, 'the payment provider failed'))).toBe(
      'the payment provider failed',
    )
  })

  it('is nothing when nothing failed', () => {
    expect(accountRefusal(null)).toBeUndefined()
  })
})

describe('the payment method refusal', () => {
  it('says billing is off on a 503', () => {
    expect(paymentMethodRefusal(new ApiRequestError(503, 'HTTP 503: Service Unavailable'))).toBe(
      BILLING_OFF,
    )
    expect(BILLING_OFF).toBe('Billing is not enabled on this installation.')
  })

  it('is the message of any other failure', () => {
    expect(paymentMethodRefusal(new ApiRequestError(502, 'the payment provider failed'))).toBe(
      'the payment provider failed',
    )
  })

  it('is nothing when nothing failed', () => {
    expect(paymentMethodRefusal(null)).toBeUndefined()
  })
})

describe('the load failure', () => {
  it('is nothing on a 404, which only says there is nothing yet', () => {
    expect(loadFailure(new ApiRequestError(404, 'no billing account'))).toBeUndefined()
  })

  it('is the message of any other failure', () => {
    expect(loadFailure(new ApiRequestError(403, 'you may not view this'))).toBe(
      'you may not view this',
    )
  })

  it('is nothing when nothing failed', () => {
    expect(loadFailure(null)).toBeUndefined()
  })
})
