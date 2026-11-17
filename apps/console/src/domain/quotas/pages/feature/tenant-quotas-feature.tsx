import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { useGetTenantQuotas, useSetTenantQuota } from '../../hooks/use-quotas'
import type { Quota, QuotaKind } from '../../quota'
import { TenantQuotas } from '../ui/tenant-quotas'

export default function TenantQuotasFeature({ organisationId }: { organisationId: string }) {
  const quotas = useGetTenantQuotas(organisationId)
  const setQuota = useSetTenantQuota()
  const canChange = useHoldsPlatformRight('act_on_tenant')

  const change = (kind: QuotaKind, quota: Quota | null) => {
    setQuota.mutate({
      path: { organisation_id: organisationId, kind },
      body: { quota },
    })
  }

  return (
    <TenantQuotas
      quotas={quotas.data?.data ?? []}
      isLoading={quotas.isLoading}
      canChange={canChange}
      pending={setQuota.isPending ? (setQuota.variables?.path.kind as QuotaKind) : undefined}
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
