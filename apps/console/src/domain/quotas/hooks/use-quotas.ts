import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import {
  deploymentQuotasQuery,
  setTenantQuotaMutation,
  tenantQuotasQuery,
} from '../services/quotas'

export const useGetTenantQuotas = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...tenantQuotasQuery(organisationId ?? 'current').queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export const useSetTenantQuota = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...setTenantQuotaMutation().mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({
        queryKey: tenantQuotasQuery(variables.path.organisation_id).queryKey,
      })
    },
  })
}

export const useGetDeploymentQuotas = (
  organisationId: string | null,
  deploymentId: string | null
) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...deploymentQuotasQuery(organisationId ?? 'current', deploymentId ?? 'current').queryOptions,
    enabled: !!organisationId && !!deploymentId && !!accessToken,
  })
}
