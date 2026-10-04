import { describe, expect, it } from 'vitest'
import { ApiRequestError } from '@/api/api.fetch'
import { describeEstimate, estimateRefusal } from './estimate'

describe('the estimate', () => {
  it('reads as a range billed by the provider', () => {
    expect(describeEstimate({ min: 4500, max: 9000 })).toBe(
      'between €45.00 and €90.00 per month, billed by your cloud provider; egress not included',
    )
  })

  it('does not pretend a range when there is none', () => {
    expect(describeEstimate({ min: 4500, max: 4500 })).toBe(
      '€45.00 per month, billed by your cloud provider; egress not included',
    )
  })

  it('carries the refusal of an invalid profile', () => {
    expect(estimateRefusal(new ApiRequestError(422, 'a dev profile takes the mutualized control plane'))).toBe(
      'a dev profile takes the mutualized control plane',
    )
  })

  it('leaves other failures to the caller', () => {
    expect(estimateRefusal(new ApiRequestError(500, 'boom'))).toBeUndefined()
    expect(estimateRefusal(null)).toBeUndefined()
  })
})
