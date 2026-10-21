import { useParams } from '@tanstack/react-router'
import { useGetDeployment, useGetDeploymentActions } from '@/api/deployment.api'
import { useGetDeploymentUptime } from '@/api/deployment-uptime.api'
import {
  useGetBillingAccount,
  useGetSubscription,
  useGetTiers,
  usePutSubscription,
} from '@/api/billing.api'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
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

  const organisationId = useResolvedOrganisationId() ?? null
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

  const { can } = useMyPermissions()
  const canManage = can(CAN.manageBilling)
  const subscription = useGetSubscription(organisationId, deploymentId ?? null)
  const tiers = useGetTiers(organisationId)
  const account = useGetBillingAccount(organisationId, can(CAN.viewBilling))
  const change = usePutSubscription()

  const changeTier = (tier: string) => {
    if (!organisationId || !deploymentId || change.isPending) return

    change.mutate({
      path: { organisation_id: organisationId, deployment_id: deploymentId },
      body: { tier },
    })
  }

  return (
    <PageDeploymentDetail
      deployment={deployment.data?.data}
      actions={filteredActions}
      availability={availability}
      subscription={{
        subscription: subscription.data,
        isLoading: subscription.isLoading,
        isError: subscription.isError,
        tiers: tiers.data?.data,
        access: { canManage, account: account.data ?? null },
        changing: change.isPending ? (change.variables?.body.tier ?? null) : null,
        error: change.error?.message,
        onChange: changeTier,
      }}
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
