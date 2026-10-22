import { describe, expect, it } from 'vitest'
import { offersCloudProviders } from './availability'

describe('whether an installation connects a cloud provider', () => {
  it('does when the platform lists one', () => {
    expect(offersCloudProviders({ data: ['scaleway'] })).toBe(true)
  })

  it('does not when the platform lists none', () => {
    expect(offersCloudProviders({ data: [] })).toBe(false)
  })

  it('does not before the platform has answered, nor when it did not', () => {
    expect(offersCloudProviders(undefined)).toBe(false)
    expect(offersCloudProviders(null)).toBe(false)
  })
})
