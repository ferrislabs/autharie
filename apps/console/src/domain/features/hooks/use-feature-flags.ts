import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import {
  enabledFeaturesQuery,
  platformFeaturesQuery,
  setFeatureDefaultMutation,
  setTenantFeatureMutation,
  tenantFeaturesQuery,
} from '../services/feature-flags'

export const useGetPlatformFeatures = () => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...platformFeaturesQuery().queryOptions,
    enabled: !!accessToken,
  })
}

export const useSetFeatureDefault = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...setFeatureDefaultMutation().mutationOptions,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: platformFeaturesQuery().queryKey })
    },
  })
}

export const useGetTenantFeatures = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...tenantFeaturesQuery(organisationId ?? 'current').queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export const useSetTenantFeature = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...setTenantFeatureMutation().mutationOptions,
    onSuccess: async (_, variables) => {
      const organisationId = variables.path.organisation_id

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: tenantFeaturesQuery(organisationId).queryKey }),
        queryClient.invalidateQueries({ queryKey: enabledFeaturesQuery(organisationId).queryKey }),
        queryClient.invalidateQueries({ queryKey: platformFeaturesQuery().queryKey }),
      ])
    },
  })
}
