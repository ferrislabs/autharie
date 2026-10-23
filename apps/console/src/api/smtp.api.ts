import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { isPending } from '@/domain/branding/smtp-state'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

interface Scope {
  organisationId: string | null
  deploymentId: string | null
}

const BASE = '/organisations/{organisation_id}/deployments/{deployment_id}/branding'
const POLL_MS = 3000

function pathOf({ organisationId, deploymentId }: Scope) {
  return { organisation_id: organisationId ?? 'current', deployment_id: deploymentId ?? 'current' }
}

function useEnabled({ organisationId, deploymentId }: Scope) {
  const accessToken = useAuthStore(selectAccessToken)
  return !!organisationId && !!deploymentId && !!accessToken
}

function smtpKey(path: { organisation_id: string; deployment_id: string; realm: string }) {
  return window.api.get(`${BASE}/{realm}/smtp`, { path }).queryKey
}

export const useGetSmtp = (scope: Scope, realm: string) => {
  const enabled = useEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/{realm}/smtp`, { path: { ...pathOf(scope), realm } }).queryOptions,
    enabled,
    refetchInterval: (query) => (isPending(query.state.data?.data?.state) ? POLL_MS : false),
  })
}

export const useGetBrandingRealms = (scope: Scope) => {
  const enabled = useEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/realms`, { path: pathOf(scope) }).queryOptions,
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
