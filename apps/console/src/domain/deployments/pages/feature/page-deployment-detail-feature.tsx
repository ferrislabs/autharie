import { useState } from 'react'
import { useParams } from '@tanstack/react-router'
import {
  useGetDeployment,
  useGetDeploymentActions,
  useRevealTenantAdministrator,
} from '@/api/deployment.api'
import { useGetDeploymentUptime } from '@/api/deployment-uptime.api'
import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { ClusterPanelFeature } from '@/domain/cloud-providers/pages/feature/cluster-panel-feature'
import {
  currentCredentialId,
  currentProfile,
  isResizable,
} from '@/domain/cloud-providers/resize'
import { availabilityView } from '../../availability'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { type RevealedAdministrator, tenantRealmView } from '../../tenant-realm'
import { TenantAdministratorPanel } from '../ui/components/tenant-administrator-panel'
import { PageDeploymentDetail } from '../ui/page-deployment-detail'

export default function PageDeploymentDetailFeature() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }

  const deployment = useGetDeployment(deploymentId ?? null)
  const actions = useGetDeploymentActions(deploymentId ?? null)
  const organisationId = useResolvedOrganisationId()
  const reveal = useRevealTenantAdministrator()
  const [revealed, setRevealed] = useState<RevealedAdministrator>()

  const canViewEstate = useHoldsPlatformRight('view_estate')
  const uptime = useGetDeploymentUptime(deploymentId ?? null, canViewEstate)
  const availability = availabilityView(canViewEstate, uptime)

  const filteredActions = actions.data?.data
    .filter((action) => {
      if (action.action_type.startsWith('system.')) {
        return false
      }
      const source = action.metadata.source
      if (typeof source === 'string' && source === 'System') {
        return false
      }
      if (typeof source === 'object' && 'System' in source) {
        return false
      }
      return true
    })
    .sort((a, b) => {
      const dateA = new Date(a.metadata.created_at).getTime()
      const dateB = new Date(b.metadata.created_at).getTime()
      return dateB - dateA
    }) ?? []

  const subject = deployment.data?.data
  const profile = subject && currentProfile(subject.distribution)
  const credentialId = subject && currentCredentialId(subject.distribution)
  const resizable =
    subject && profile && credentialId && isResizable(subject, deployment.data?.provisioning)

  const realm = tenantRealmView(deployment.data?.realm)
  const onReveal = () => {
    if (!organisationId || !deploymentId) return
    reveal.mutate(
      { path: { organisation_id: organisationId, deployment_id: deploymentId } },
      { onSuccess: async (answer) => setRevealed((await answer.json()).data) }
    )
  }

  return (
    <PageDeploymentDetail
      deployment={deployment.data?.data}
      actions={filteredActions}
      availability={availability}
      provisioning={deployment.data?.provisioning}
      realm={realm}
      administratorPanel={
        realm?.administrator ? (
          <TenantAdministratorPanel
            administrator={realm.administrator}
            revealed={revealed}
            revealing={reveal.isPending}
            error={reveal.error instanceof Error ? reveal.error.message : undefined}
            onReveal={onReveal}
          />
        ) : undefined
      }
      clusterPanel={
        resizable ? (
          <ClusterPanelFeature
            key={subject.id}
            deploymentId={subject.id}
            credentialId={credentialId}
            profile={profile}
          />
        ) : undefined
      }
      isLoading={deployment.isLoading}
    />
  )
}
