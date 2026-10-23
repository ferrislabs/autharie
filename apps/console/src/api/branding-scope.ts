import { selectAccessToken, useAuthStore } from '@/stores/auth'

export interface BrandingScope {
  organisationId: string | null
  deploymentId: string | null
}

export const BRANDING_POLL_MS = 3000

export function brandingPath({ organisationId, deploymentId }: BrandingScope) {
  return { organisation_id: organisationId ?? 'current', deployment_id: deploymentId ?? 'current' }
}

export function useBrandingEnabled({ organisationId, deploymentId }: BrandingScope) {
  const accessToken = useAuthStore(selectAccessToken)
  return !!organisationId && !!deploymentId && !!accessToken
}
