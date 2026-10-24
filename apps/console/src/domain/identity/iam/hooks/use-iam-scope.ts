import { useMemo } from 'react'
import { useParams } from '@tanstack/react-router'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import type { IamScope } from '../types'

export function useIamScope(): IamScope {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }
  const organisationId = useResolvedOrganisationId()

  return useMemo(
    () => ({ organisationId: organisationId ?? null, deploymentId: deploymentId ?? null }),
    [organisationId, deploymentId],
  )
}
