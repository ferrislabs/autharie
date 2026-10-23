import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { isPending } from '@/domain/branding/smtp-state'
import {
  BRANDING_POLL_MS,
  brandingPath,
  useBrandingEnabled,
  type BrandingScope,
} from '@/api/branding-scope'

const BASE = '/organisations/{organisation_id}/deployments/{deployment_id}/branding'

function smtpKey(path: { organisation_id: string; deployment_id: string; realm: string }) {
  return window.api.get(`${BASE}/{realm}/smtp`, { path }).queryKey
}

export const useGetSmtp = (scope: BrandingScope, realm: string) => {
  const enabled = useBrandingEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/{realm}/smtp`, { path: { ...brandingPath(scope), realm } }).queryOptions,
    enabled,
    refetchInterval: (query) => (isPending(query.state.data?.data?.state) ? BRANDING_POLL_MS : false),
  })
}

export const useGetBrandingRealms = (scope: BrandingScope) => {
  const enabled = useBrandingEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/realms`, { path: brandingPath(scope) }).queryOptions,
    enabled,
  })
}

export const useSaveSmtp = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('put', `${BASE}/{realm}/smtp`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: smtpKey(variables.path) }),
  })
}

export const useRemoveSmtp = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('delete', `${BASE}/{realm}/smtp`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: smtpKey(variables.path) }),
  })
}
