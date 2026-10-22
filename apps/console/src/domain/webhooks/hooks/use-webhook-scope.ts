import { useMemo } from 'react'
import { useParams } from '@tanstack/react-router'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'

export function useWebhookScope() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }
  const organisationId = useResolvedOrganisationId()

  return useMemo(
    () => ({ organisationId: organisationId ?? null, deploymentId: deploymentId ?? null }),
    [organisationId, deploymentId]
  )
}
