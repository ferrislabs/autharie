import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

function accountKey(organisationId: string) {
  return window.api.get('/organisations/{organisation_id}/billing/account', {
    path: { organisation_id: organisationId },
  }).queryKey
}

function subscriptionKey(organisationId: string, deploymentId: string) {
  return window.api.get('/organisations/{organisation_id}/deployments/{deployment_id}/subscription', {
    path: { organisation_id: organisationId, deployment_id: deploymentId },
  }).queryKey
}

export const useGetTiers = (organisationId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/billing/tiers', {
      path: { organisation_id: organisationId ?? 'current' },
    }).queryOptions,
    enabled: !!organisationId && !!accessToken,
  })
}

export const useGetBillingAccount = (organisationId: string | null, enabled = true) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/billing/account', {
      path: { organisation_id: organisationId ?? 'current' },
    }).queryOptions,
    enabled: enabled && !!organisationId && !!accessToken,
    retry: false,
  })
}

export const usePutBillingAccount = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('put', '/organisations/{organisation_id}/billing/account').mutationOptions,
    onSuccess: async (_, variables) => {
      await queryClient.invalidateQueries({
        queryKey: accountKey(variables.path.organisation_id),
      })
    },
  })
}

export const useCreatePaymentMethodSession = () =>
  useMutation({
    ...window.api.mutation('post', '/organisations/{organisation_id}/billing/payment-method-session')
      .mutationOptions,
  })

export const useGetStatements = (organisationId: string | null, enabled = true) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/billing/statements', {
      path: { organisation_id: organisationId ?? 'current' },
    }).queryOptions,
    enabled: enabled && !!organisationId && !!accessToken,
  })
}

export const useGetStatement = (organisationId: string | null, period: string, enabled = true) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/billing/statements/{period}', {
      path: { organisation_id: organisationId ?? 'current', period },
    }).queryOptions,
    enabled: enabled && !!organisationId && !!accessToken,
  })
}

export const useGetSubscription = (organisationId: string | null, deploymentId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/deployments/{deployment_id}/subscription', {
      path: {
        organisation_id: organisationId ?? 'current',
        deployment_id: deploymentId ?? 'current',
      },
    }).queryOptions,
    enabled: !!organisationId && !!deploymentId && !!accessToken,
  })
}

export const usePutSubscription = () => {
  const queryClient = useQueryClient()

  return useMutation({
    ...window.api.mutation('put', '/organisations/{organisation_id}/deployments/{deployment_id}/subscription')
      .mutationOptions,
    onSuccess: async (_, variables) => {
      const { organisation_id, deployment_id } = variables.path
      await queryClient.invalidateQueries({
        queryKey: subscriptionKey(organisation_id, deployment_id),
      })
    },
  })
}

export const useGetDeploymentFeatures = (organisationId: string | null, deploymentId: string | null) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/deployments/{deployment_id}/features', {
      path: {
        organisation_id: organisationId ?? 'current',
        deployment_id: deploymentId ?? 'current',
      },
    }).queryOptions,
    enabled: !!organisationId && !!deploymentId && !!accessToken,
  })
}
