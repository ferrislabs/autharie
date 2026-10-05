import { useParams } from '@tanstack/react-router'
import { useGetDeployment, useGetDeploymentActions } from '@/api/deployment.api'
import { useGetDeploymentUptime } from '@/api/deployment-uptime.api'
import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { ClusterPanelFeature } from '@/domain/cloud-providers/pages/feature/cluster-panel-feature'
import {
  currentCredentialId,
  currentProfile,
  isResizable,
} from '@/domain/cloud-providers/resize'
import { availabilityView } from '../../availability'
import { PageDeploymentDetail } from '../ui/page-deployment-detail'

export default function PageDeploymentDetailFeature() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }

  const deployment = useGetDeployment(deploymentId ?? null)
  const actions = useGetDeploymentActions(deploymentId ?? null)

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

  return (
    <PageDeploymentDetail
      deployment={deployment.data?.data}
      actions={filteredActions}
      availability={availability}
      provisioning={deployment.data?.provisioning}
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
