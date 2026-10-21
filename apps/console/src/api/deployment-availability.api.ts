import { useQuery } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import { fetcher } from './api.fetch'

export interface AvailabilityDayResponse {
  day: string
  checks: number
  failed: number
  uptime: number | null
  down_seconds: number
}

export interface DeploymentAvailabilityResponse {
  data: {
    days: AvailabilityDayResponse[]
    uptime: number | null
    probe_interval_seconds: number
  }
}

export const useGetDeploymentAvailability = (
  organisationId: string | null,
  deploymentId: string | null,
  days = 90,
) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    queryKey: ['deployment-availability', organisationId, deploymentId, days],
    queryFn: async (): Promise<DeploymentAvailabilityResponse> => {
      const response = await fetcher(
        'get',
        `${window.apiUrl ?? ''}/organisations/{organisation_id}/deployments/{deployment_id}/availability`,
        {
          path: { organisation_id: organisationId, deployment_id: deploymentId },
          query: { days },
        },
      )
      return response.json()
    },
    enabled: !!organisationId && !!deploymentId && !!accessToken,
    retry: false,
  })
}
