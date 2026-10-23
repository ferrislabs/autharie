import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  BRANDING_POLL_MS,
  brandingPath,
  useBrandingEnabled,
  type BrandingScope,
} from '@/api/branding-scope'
import { isApplying } from '@/domain/branding/theme-state'

const THEMES =
  '/organisations/{organisation_id}/deployments/{deployment_id}/branding/{realm}/themes'

interface RealmPath {
  organisation_id: string
  deployment_id: string
  realm: string
}

function themesKey({ organisation_id, deployment_id, realm }: RealmPath) {
  return window.api.get(THEMES, { path: { organisation_id, deployment_id, realm } }).queryKey
}

export const useGetThemes = (scope: BrandingScope, realm: string) => {
  const enabled = useBrandingEnabled(scope)

  return useQuery({
    ...window.api.get(THEMES, { path: { ...brandingPath(scope), realm } }).queryOptions,
    enabled,
    refetchInterval: (query) => (isApplying(query.state.data?.state) ? BRANDING_POLL_MS : false),
  })
}

export const useCreateTheme = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', THEMES, (response) => response.json()).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: themesKey(variables.path) }),
  })
}

export const useUpdateTheme = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('put', `${THEMES}/{theme_id}`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: themesKey(variables.path) }),
  })
}

export const useDeleteTheme = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('delete', `${THEMES}/{theme_id}`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: themesKey(variables.path) }),
  })
}

export const useActivateTheme = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', `${THEMES}/{theme_id}/activate`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: themesKey(variables.path) }),
  })
}

export const useDeactivateTheme = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', `${THEMES}/deactivate`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: themesKey(variables.path) }),
  })
}
