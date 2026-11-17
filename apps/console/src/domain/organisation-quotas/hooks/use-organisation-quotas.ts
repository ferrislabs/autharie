import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import {
  organisationQuotasQuery,
  setOrganisationQuotaMutation,
} from '../services/organisation-quotas'

export const useGetOrganisationQuotas = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...organisationQuotasQuery(organisationId ?? 'current').queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export const useSetOrganisationQuota = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...setOrganisationQuotaMutation().mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({
        queryKey: organisationQuotasQuery(variables.path.organisation_id).queryKey,
      })
    },
  })
}
