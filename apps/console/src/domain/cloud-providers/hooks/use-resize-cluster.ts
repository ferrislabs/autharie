import { useMutation, useQueryClient } from '@tanstack/react-query'

export const useResizeCluster = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation(
      'put',
      '/organisations/{organisation_id}/deployments/{deployment_id}/cluster-profile',
      (response) => response.json(),
    ).mutationOptions,
    onSuccess: async (_, variables) => {
      const { organisation_id, deployment_id } = variables.path

      await queryClient.invalidateQueries({
        queryKey: window.api.get('/organisations/{organisation_id}/deployments/{deployment_id}', {
          path: { organisation_id, deployment_id },
        }).queryKey,
      })
    },
  })
}
