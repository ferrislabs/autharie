import { useParams } from '@tanstack/react-router'
import { useGetDeploymentFeatures } from '@/api/billing.api'
import { useGetIamSettings, useSetIamSettings } from '@/api/iam-settings.api'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { isBrandingOpen, toApplyRequest, toResetRequest } from '../../branding'
import { PageBranding } from '../ui/page-branding'

export default function PageBrandingFeature() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }
  const organisationId = useResolvedOrganisationId()

  const features = useGetDeploymentFeatures(organisationId ?? null, deploymentId ?? null)
  const settings = useGetIamSettings(organisationId ?? null, deploymentId ?? null)
  const save = useSetIamSettings()

  const open = isBrandingOpen(features.data?.features)

  const send = (body: Parameters<typeof save.mutate>[0]['body']) => {
    if (!organisationId || !deploymentId || save.isPending) return

    save.mutate({
      path: { organisation_id: organisationId, deployment_id: deploymentId },
      body,
    })
  }

  return (
    <PageBranding
      isLoading={features.isLoading || (open && settings.isLoading)}
      feature={{ feature: 'branding', open }}
      tier={features.data?.tier}
      state={settings.data?.data}
      isSaving={save.isPending}
      error={save.error?.message}
      onApply={(form) => send(toApplyRequest(form))}
      onReset={() => send(toResetRequest())}
    />
  )
}
