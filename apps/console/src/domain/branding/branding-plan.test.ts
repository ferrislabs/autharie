import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { describeUnavailable, isBrandingOpen } from './branding-plan'

const features = (branding: boolean): Schemas.DeploymentFeature[] => [
  { feature: 'mfa', open: true },
  { feature: 'branding', open: branding },
]

describe('branding on a deployment', () => {
  it('is open when the features of the deployment open it', () => {
    expect(isBrandingOpen(features(true))).toBe(true)
  })

  it('is closed when the features of the deployment close it', () => {
    expect(isBrandingOpen(features(false))).toBe(false)
  })

  it('is closed when the features do not mention it', () => {
    expect(isBrandingOpen([{ feature: 'mfa', open: true }])).toBe(false)
  })

  it('is closed while the features are unknown', () => {
    expect(isBrandingOpen(undefined)).toBe(false)
  })
})

describe('saying why branding is closed', () => {
  it('names the tier', () => {
    expect(describeUnavailable('Free')).toBe(
      'Branding is not included in the Free tier of this deployment.'
    )
  })

  it('does without a tier', () => {
    expect(describeUnavailable(undefined)).toBe(
      'Branding is not included in the tier of this deployment.'
    )
    expect(describeUnavailable(null)).toBe(
      'Branding is not included in the tier of this deployment.'
    )
  })
})
