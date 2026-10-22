import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { Schemas } from '@/api/api.client'
import { DELIVERIES_REFRESH_MS } from '@/domain/webhooks/deliveries'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

interface Scope {
  organisationId: string | null
  deploymentId: string | null
}

const BASE = '/organisations/{organisation_id}/deployments/{deployment_id}'

function pathOf({ organisationId, deploymentId }: Scope) {
  return { organisation_id: organisationId ?? 'current', deployment_id: deploymentId ?? 'current' }
}

function useEnabled({ organisationId, deploymentId }: Scope) {
  const accessToken = useAuthStore(selectAccessToken)
  return !!organisationId && !!deploymentId && !!accessToken
}

function webhooksKey(scope: Scope) {
  return window.api.get(`${BASE}/webhooks`, { path: pathOf(scope) }).queryKey
}

function webhookKey(scope: Scope, webhookId: string) {
  return window.api.get(`${BASE}/webhooks/{webhook_id}`, {
    path: { ...pathOf(scope), webhook_id: webhookId },
  }).queryKey
}

export const useGetWebhooks = (scope: Scope) => {
  const enabled = useEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/webhooks`, { path: pathOf(scope) }).queryOptions,
    enabled,
    refetchInterval: (query) =>
      query.state.data?.data.some((webhook) => webhook.state.phase === 'Pending') ? 3000 : false,
  })
}

export const useGetWebhook = (
  scope: Scope,
  webhookId: string | null,
  pollEvery: (webhook?: Schemas.WebhookView) => number | false
) => {
  const enabled = useEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/webhooks/{webhook_id}`, {
      path: { ...pathOf(scope), webhook_id: webhookId ?? 'current' },
    }).queryOptions,
    enabled: enabled && !!webhookId,
    refetchInterval: (query) => pollEvery(query.state.data?.data),
  })
}

export const useGetWebhookRealms = (scope: Scope) => {
  const enabled = useEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/webhook-realms`, { path: pathOf(scope) }).queryOptions,
    enabled,
  })
}

export const useGetWebhookEvents = () => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/webhook-events').queryOptions,
    enabled: !!accessToken,
    staleTime: 60 * 60 * 1000,
  })
}

export const useGetWebhookDeliveries = (
  scope: Scope,
  webhookId: string | null,
  status: string | undefined
) => {
  const enabled = useEnabled(scope)

  return useQuery({
    ...window.api.get(`${BASE}/webhooks/{webhook_id}/deliveries`, {
      path: { ...pathOf(scope), webhook_id: webhookId ?? 'current' },
      query: status ? { status } : {},
    }).queryOptions,
    enabled: enabled && !!webhookId,
    refetchInterval: DELIVERIES_REFRESH_MS,
  })
}

export const useCreateWebhook = (scope: Scope) => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', `${BASE}/webhooks`).mutationOptions,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: webhooksKey(scope) }),
  })
}

export const useUpdateWebhook = (scope: Scope) => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('put', `${BASE}/webhooks/{webhook_id}`).mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({ queryKey: webhooksKey(scope) })
      await queryClient.invalidateQueries({
        queryKey: webhookKey(scope, variables.path.webhook_id),
      })
    },
  })
}

export const useDeleteWebhook = (scope: Scope) => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('delete', `${BASE}/webhooks/{webhook_id}`).mutationOptions,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: webhooksKey(scope) }),
  })
}

export const useRotateWebhookSecret = (scope: Scope) => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('post', `${BASE}/webhooks/{webhook_id}/secret/rotate`).mutationOptions,
    onSuccess: (_, variables) =>
      queryClient.invalidateQueries({ queryKey: webhookKey(scope, variables.path.webhook_id) }),
  })
}

export const useRevealWebhookSecret = () =>
  useMutation({
    ...window.api.get(`${BASE}/webhooks/{webhook_id}/secret`, {
      path: { organisation_id: '', deployment_id: '', webhook_id: '' },
    }).mutationOptions,
  })

export const useRetryWebhookDelivery = () =>
  useMutation({
    ...window.api.mutation('post', `${BASE}/webhooks/{webhook_id}/deliveries/{delivery_id}/retry`)
      .mutationOptions,
  })
