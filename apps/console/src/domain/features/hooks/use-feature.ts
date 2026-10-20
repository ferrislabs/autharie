import { useQuery } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import { isFeatureEnabled } from '../choice'
import { enabledFeaturesQuery } from '../services/feature-flags'
import type { Feature } from '../types/feature'

export const useEnabledFeatures = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...enabledFeaturesQuery(organisationId ?? 'current').queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export function useFeature(organisationId: string | null, key: Feature): boolean {
  const { data } = useEnabledFeatures(organisationId)

  return isFeatureEnabled(data?.enabled, key)
}
