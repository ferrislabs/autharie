import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { useGetPlatformFeatures, useSetFeatureDefault } from '../../hooks/use-feature-flags'
import type { Feature } from '../../types/feature'
import { PageFeatures } from '../ui/page-features'

export default function PageFeaturesFeature() {
  const features = useGetPlatformFeatures()
  const setDefault = useSetFeatureDefault()
  const canChange = useHoldsPlatformRight('operate_fleet')

  const toggle = (feature: Feature, enabled: boolean) => {
    setDefault.mutate({ path: { key: feature }, body: { enabled_by_default: enabled } })
  }

  return (
    <PageFeatures
      features={features.data?.data ?? []}
      isLoading={features.isLoading}
      canChange={canChange}
      pending={setDefault.isPending ? (setDefault.variables?.path.key as Feature) : undefined}
      refusal={
        setDefault.isError
          ? setDefault.error instanceof Error
            ? setDefault.error.message
            : 'The default could not be changed.'
          : undefined
      }
      onToggle={toggle}
    />
  )
}
