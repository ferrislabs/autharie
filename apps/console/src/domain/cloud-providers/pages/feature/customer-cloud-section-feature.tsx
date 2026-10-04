import { useEffect, useMemo, useState } from 'react'
import type { Schemas } from '@/api/api.client'
import { REGIONS, toDistributionRequest } from '../../distribution'
import { describeEstimate, estimateRefusal } from '../../estimate'
import { useDebouncedValue } from '../../hooks/use-debounced-value'
import { useEstimateClusterProfile, useGetProviderOffers } from '../../hooks/use-cluster-planning'
import {
  applyMode,
  initialDraft,
  isComplete,
  setMaxNodes,
  setMinNodes,
  toProfileRequest,
  type ProfileDraft,
} from '../../profile'
import type { CloudCredential } from '../../types/cloud-provider'
import { CustomerCloudForm } from '../ui/customer-cloud-form'

const ESTIMATE_DEBOUNCE_MS = 400

interface Props {
  credentials: CloudCredential[]
  onChange: (distribution: Schemas.DistributionRequest | null) => void
}

export function CustomerCloudSectionFeature({ credentials, onChange }: Props) {
  const [credentialId, setCredentialId] = useState(credentials[0]?.id ?? '')
  const credential = credentials.find((entry) => entry.id === credentialId) ?? credentials[0]
  const regions = REGIONS[credential.provider]
  const [chosenRegion, setRegion] = useState(regions[0])
  const region = regions.includes(chosenRegion) ? chosenRegion : regions[0]
  const [edited, setEdited] = useState<ProfileDraft | null>(null)

  const offers = useGetProviderOffers(credential.id, region)
  const catalogue = offers.data
  const initial = useMemo(() => (catalogue ? initialDraft(catalogue) : null), [catalogue])
  const draft = edited ?? initial
  const complete = !!catalogue && !!draft && isComplete(draft, catalogue)

  const request = useMemo(
    () =>
      complete && draft
        ? { credential_id: credential.id, region, ...toProfileRequest(draft) }
        : null,
    [complete, draft, credential.id, region],
  )
  const settled = useDebouncedValue(request, ESTIMATE_DEBOUNCE_MS)
  const estimate = useEstimateClusterProfile(settled)

  const distribution = useMemo(
    () =>
      complete && draft
        ? toDistributionRequest({ credentialId: credential.id, region, profile: draft })
        : null,
    [complete, draft, credential.id, region],
  )

  useEffect(() => {
    onChange(distribution)
  }, [distribution, onChange])

  const refusal = estimateRefusal(estimate.error)

  const edit = (change: (current: ProfileDraft) => ProfileDraft) => {
    if (draft) setEdited(change(draft))
  }

  return (
    <CustomerCloudForm
      credentials={credentials}
      credentialId={credential.id}
      onCredential={(id) => {
        setCredentialId(id)
        setEdited(null)
      }}
      regions={regions}
      region={region}
      onRegion={(next) => {
        setRegion(next)
        setEdited(null)
      }}
      offers={catalogue}
      offersLoading={offers.isLoading}
      offersFailed={offers.error instanceof Error ? offers.error.message : undefined}
      draft={draft}
      onMode={(mode) => catalogue && edit((current) => applyMode(current, mode, catalogue))}
      onControlPlane={(id) => edit((current) => ({ ...current, controlPlaneId: id }))}
      onNodeType={(nodeType) => edit((current) => ({ ...current, nodeType }))}
      onMinNodes={(value) => edit((current) => setMinNodes(current, value))}
      onMaxNodes={(value) => edit((current) => setMaxNodes(current, value))}
      estimate={{
        loading: request !== null && (estimate.isFetching || settled !== request),
        text: !refusal && estimate.data && settled === request ? describeEstimate(estimate.data) : undefined,
        refusal: settled === request ? refusal : undefined,
      }}
    />
  )
}
