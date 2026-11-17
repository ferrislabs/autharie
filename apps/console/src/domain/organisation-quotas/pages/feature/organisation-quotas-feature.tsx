import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import type { Quota } from '@/domain/quotas/quota'
import {
  useGetOrganisationQuotas,
  useSetOrganisationQuota,
} from '../../hooks/use-organisation-quotas'
import type { OrganisationQuotaKind } from '../../types'
import { OrganisationQuotas } from '../ui/organisation-quotas'

export default function OrganisationQuotasFeature({ organisationId }: { organisationId: string }) {
  const quotas = useGetOrganisationQuotas(organisationId)
  const setQuota = useSetOrganisationQuota()
  const canChange = useHoldsPlatformRight('act_on_tenant')

  const change = (kind: OrganisationQuotaKind, quota: Quota | null) => {
    setQuota.mutate({
      path: { organisation_id: organisationId, kind },
      body: { quota },
    })
  }

  return (
    <OrganisationQuotas
      quotas={quotas.data?.data ?? []}
      isLoading={quotas.isLoading}
      canChange={canChange}
      pending={
        setQuota.isPending ? (setQuota.variables?.path.kind as OrganisationQuotaKind) : undefined
      }
      refusal={
        setQuota.isError
          ? setQuota.error instanceof Error
            ? setQuota.error.message
            : 'The quota could not be changed.'
          : undefined
      }
      onChange={change}
    />
  )
}
