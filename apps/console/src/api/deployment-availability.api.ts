import { useQuery } from '@tanstack/react-query'
import type { Schemas } from '@/api/api.client'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

export type AvailabilityDayResponse = Schemas.AvailabilityHistory['days'][number]

/**
 * A deployment's availability, day by day, for its own organisation.
 *
 * Not retried: a 404 means nothing has been checked yet, and asking again
 * three times only delays the panel saying so.
 */
export const useGetDeploymentAvailability = (
  organisationId: string | null,
  deploymentId: string | null,
  days = 90,
) => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/organisations/{organisation_id}/deployments/{deployment_id}/availability', {
      path: {
        organisation_id: organisationId ?? 'current',
        deployment_id: deploymentId ?? 'current',
      },
      query: { days },
    }).queryOptions,
    enabled: !!organisationId && !!deploymentId && !!accessToken,
    retry: false,
  })
}
