import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

export const useGetCloudCredentials = () => {
  const organisationId = useResolvedOrganisationId()
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/cloud-credentials', {
      path: { organisation_id: organisationId ?? 'current' },
    }).queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

const credentialsKey = (organisationId: string) =>
  window.api.get('/organisations/{organisation_id}/cloud-credentials', {
    path: { organisation_id: organisationId },
  }).queryKey

export const useRegisterCloudCredential = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', '/organisations/{organisation_id}/cloud-credentials')
      .mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({
        queryKey: credentialsKey(variables.path.organisation_id),
      })
    },
  })
}

export const useDeleteCloudCredential = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation(
      'delete',
      '/organisations/{organisation_id}/cloud-credentials/{credential_id}',
    ).mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({
        queryKey: credentialsKey(variables.path.organisation_id),
      })
    },
  })
}
