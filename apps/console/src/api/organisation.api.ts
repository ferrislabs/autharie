import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useAuthStore, selectAccessToken } from '@/stores/auth'

export const useGetUserOrganisations = () => {
  const accessToken = useAuthStore(selectAccessToken)
  return useQuery({
    ...window.api.get('/users/@me/organisations').queryOptions,
    enabled: !!accessToken,
  })
}

export const useCreateOrganisation = () => {
  return useMutation({
    ...window.api.mutation('post', '/organisations').mutationOptions,
  })
}

function organisationKey(organisationId: string) {
  return window.api.get('/organisations/{organisation_id}', {
    path: { organisation_id: organisationId },
  }).queryKey
}

export const useGetOrganisation = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}', {
      path: { organisation_id: organisationId ?? 'current' },
    }).queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export const useUpdateOrganisation = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('patch', '/organisations/{organisation_id}').mutationOptions,
    onSuccess: async (_, variables) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: organisationKey(variables.path.organisation_id) }),
        queryClient.invalidateQueries({
          queryKey: window.api.get('/users/@me/organisations').queryKey,
        }),
      ])
    },
  })
}
