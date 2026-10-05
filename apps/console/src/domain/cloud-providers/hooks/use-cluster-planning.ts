import { useQuery } from '@tanstack/react-query'
import type { Schemas } from '@/api/api.client'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

export const useGetProviderOffers = (credentialId: string | null, region: string) => {
  const organisationId = useResolvedOrganisationId()
  const accessToken = useAuthStore(selectAccessToken)

  const params = {
    path: {
      organisation_id: organisationId ?? 'current',
      credential_id: credentialId ?? 'current',
    },
    query: { region },
  }

  return useQuery({
    ...window.api.get(
      '/organisations/{organisation_id}/cloud-credentials/{credential_id}/offers',
      params,
    ).queryOptions,
    enabled: !!organisationId && !!credentialId && !!accessToken,
  })
}

export const useEstimateClusterProfile = (
  body: Schemas.EstimateClusterProfileRequest | null,
) => {
  const organisationId = useResolvedOrganisationId()
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.post('/organisations/{organisation_id}/cluster-profiles/estimate', {
      path: { organisation_id: organisationId ?? 'current' },
      body: body as Schemas.EstimateClusterProfileRequest,
    }).queryOptions,
    enabled: !!organisationId && !!accessToken && body !== null,
  })
}
