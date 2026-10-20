import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { overrideOf } from '../../choice'
import { useGetTenantFeatures, useSetTenantFeature } from '../../hooks/use-feature-flags'
import type { Feature, OverrideChoice } from '../../types/feature'
import { TenantFeatures } from '../ui/tenant-features'

export default function TenantFeaturesFeature({ organisationId }: { organisationId: string }) {
  const features = useGetTenantFeatures(organisationId)
  const setFeature = useSetTenantFeature()
  const canChange = useHoldsPlatformRight('act_on_tenant')

  const choose = (feature: Feature, choice: OverrideChoice) => {
    setFeature.mutate({
      path: { organisation_id: organisationId, key: feature },
      body: { enabled: overrideOf(choice) },
    })
  }

  return (
    <TenantFeatures
      features={features.data?.data ?? []}
      isLoading={features.isLoading}
      canChange={canChange}
      pending={setFeature.isPending ? (setFeature.variables?.path.key as Feature) : undefined}
      refusal={
        setFeature.isError
          ? setFeature.error instanceof Error
            ? setFeature.error.message
            : 'The feature could not be changed.'
          : undefined
      }
      onChoose={choose}
    />
  )
}
