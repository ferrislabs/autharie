import { describe, expect, it } from 'vitest'
import { ApiRequestError } from '@/api/api.fetch'
import { PERIOD_UNKNOWN, estimateFailure, isFirstLoad } from './billing-load'

describe('the first load', () => {
  it('is on while a query has never answered', () => {
    expect(isFirstLoad([{ isLoading: true, isFetched: false }])).toBe(true)
  })

  it('is on as long as one query has never answered', () => {
    expect(
      isFirstLoad([
        { isLoading: false, isFetched: true },
        { isLoading: true, isFetched: false },
      ]),
    ).toBe(true)
  })

  it('is over once every query answered, whatever it answered', () => {
    expect(
      isFirstLoad([
        { isLoading: false, isFetched: true },
        { isLoading: false, isFetched: true },
      ]),
    ).toBe(false)
  })

  it('does not come back with a background refetch, even of a query that answered an error', () => {
    expect(isFirstLoad([{ isLoading: true, isFetched: true }])).toBe(false)
  })

  it('is off for a query that is not enabled', () => {
    expect(isFirstLoad([{ isLoading: false, isFetched: false }])).toBe(false)
    expect(isFirstLoad([])).toBe(false)
  })
})

describe('the estimate failure', () => {
  it('is the message of the estimate, a 404 included', () => {
    expect(estimateFailure(null, new ApiRequestError(404, 'no statement for this period'))).toBe(
      'no statement for this period',
    )
    expect(estimateFailure(null, new ApiRequestError(500, 'it broke'))).toBe('it broke')
  })

  it('says the month is unknown when the statements, which name it, could not be loaded', () => {
    expect(estimateFailure(new ApiRequestError(500, 'it broke'), null)).toBe(PERIOD_UNKNOWN)
  })

  it('prefers what the estimate said', () => {
    expect(estimateFailure(new ApiRequestError(500, 'list'), new Error('estimate'))).toBe('estimate')
  })

  it('is nothing when nothing failed', () => {
    expect(estimateFailure(null, null)).toBeUndefined()
  })
})
