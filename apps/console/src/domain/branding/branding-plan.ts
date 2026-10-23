import type { Schemas } from '@/api/api.client'

export function describeUnavailable(tier: string | null | undefined): string {
  return tier
    ? `Branding is not included in the ${tier} tier of this deployment.`
    : 'Branding is not included in the tier of this deployment.'
}

export function isBrandingOpen(
  features: ReadonlyArray<Schemas.DeploymentFeature> | undefined
): boolean {
  return features?.some((entry) => entry.feature === 'branding' && entry.open) ?? false
}
