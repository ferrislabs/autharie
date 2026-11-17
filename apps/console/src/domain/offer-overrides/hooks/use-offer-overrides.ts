import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import { offerOverridesQuery, setOfferOverrideMutation } from '../services/offer-overrides'

export const useGetOfferOverrides = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...offerOverridesQuery(organisationId ?? 'current').queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export const useSetOfferOverride = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...setOfferOverrideMutation().mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({
        queryKey: offerOverridesQuery(variables.path.organisation_id).queryKey,
      })
    },
  })
}
