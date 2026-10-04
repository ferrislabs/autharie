import { useMemo, useState } from 'react'
import type { Schemas } from '@/api/api.client'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { REGIONS } from '../../distribution'
import { describeEstimate, estimateRefusal } from '../../estimate'
import { useDebouncedValue } from '../../hooks/use-debounced-value'
import { useEstimateClusterProfile, useGetProviderOffers } from '../../hooks/use-cluster-planning'
import { useResizeCluster } from '../../hooks/use-resize-cluster'
import { applyMode, setMaxNodes, setMinNodes, toProfileRequest, type ProfileDraft } from '../../profile'
import { canApply, draftFromProfile, resizeFailure } from '../../resize'
import { ClusterPanel } from '../ui/cluster-panel'

const ESTIMATE_DEBOUNCE_MS = 400

interface Props {
  deploymentId: string
  credentialId: string
  profile: Schemas.ClusterProfile
}

export function ClusterPanelFeature({ deploymentId, credentialId, profile }: Props) {
  const organisationId = useResolvedOrganisationId()
  const regions = REGIONS.scaleway
  const [region, setRegion] = useState(regions[0])
  const [edited, setEdited] = useState<ProfileDraft | null>(null)
  const resize = useResizeCluster()

  const offers = useGetProviderOffers(credentialId, region)
  const catalogue = offers.data
  const initial = useMemo(() => draftFromProfile(profile), [profile])
  const draft = edited ?? initial
  const applicable = !!catalogue && canApply(draft, profile, catalogue)

  const request = useMemo(
    () => (applicable ? { credential_id: credentialId, region, ...toProfileRequest(draft) } : null),
    [applicable, draft, credentialId, region],
  )
  const settled = useDebouncedValue(request, ESTIMATE_DEBOUNCE_MS)
  const estimate = useEstimateClusterProfile(settled)
  const refusal = estimateRefusal(estimate.error)

  const edit = (change: (current: ProfileDraft) => ProfileDraft) => setEdited(change(draft))

  return (
    <ClusterPanel
      profile={profile}
      regions={regions}
      region={region}
      onRegion={setRegion}
      offers={catalogue}
      offersLoading={offers.isLoading}
      offersFailed={offers.error instanceof Error ? offers.error.message : undefined}
      draft={draft}
      onMode={(mode) => catalogue && edit((current) => applyMode(current, mode, catalogue))}
      onControlPlane={(id) => edit((current) => ({ ...current, controlPlaneId: id }))}
      onMinNodes={(value) => edit((current) => setMinNodes(current, value))}
      onMaxNodes={(value) => edit((current) => setMaxNodes(current, value))}
      estimate={{
        loading: request !== null && (estimate.isFetching || settled !== request),
        text: !refusal && estimate.data && settled === request ? describeEstimate(estimate.data) : undefined,
        refusal: settled === request ? refusal : undefined,
      }}
      canApply={applicable}
      applying={resize.isPending}
      failure={resizeFailure(resize.error)}
      applied={resize.data?.data}
      onApply={() => {
        if (!organisationId || resize.isPending) return

        resize.mutate(
          {
            path: { organisation_id: organisationId, deployment_id: deploymentId },
            body: toProfileRequest(draft),
          },
          { onSuccess: () => setEdited(null) },
        )
      }}
    />
  )
}
