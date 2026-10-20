import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

export const useGetCells = () => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/platform/cells').queryOptions,
    enabled: !!accessToken,
  })
}

export const useGetCellsSettings = () => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/platform/cells/settings').queryOptions,
    enabled: !!accessToken,
  })
}

export const useUpdateCellsSettings = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('put', '/platform/cells/settings').mutationOptions,
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: window.api.get('/platform/cells/settings').queryKey,
      })
    },
  })
}

const invalidateCells = async (queryClient: ReturnType<typeof useQueryClient>) => {
  await queryClient.invalidateQueries({ queryKey: window.api.get('/platform/cells').queryKey })
}

export const useCreateCell = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', '/platform/cells').mutationOptions,
    onSuccess: () => invalidateCells(queryClient),
  })
}

export const useDrainCell = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', '/platform/cells/{cell_id}/drain').mutationOptions,
    onSuccess: () => invalidateCells(queryClient),
  })
}

export const useRetireCell = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', '/platform/cells/{cell_id}/retire').mutationOptions,
    onSuccess: () => invalidateCells(queryClient),
  })
}
