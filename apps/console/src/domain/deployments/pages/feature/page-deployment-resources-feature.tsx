import { useParams } from '@tanstack/react-router'
import { useGetDeployment } from '@/api/deployment.api'
import {
  useGetBillingAccount,
  useGetSubscription,
  useGetTiers,
  usePutSubscription,
} from '@/api/billing.api'
import { accountOf } from '@/domain/billing/billing'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { CAN } from '@/domain/organisations/permissions'
import { PageDeploymentResources } from '../ui/page-deployment-resources'

export default function PageDeploymentResourcesFeature() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }
  const organisationId = useResolvedOrganisationId() ?? null
  const deployment = useGetDeployment(deploymentId ?? null)

  const { can } = useMyPermissions()
  const canManage = can(CAN.manageBilling)
  const canViewBilling = can(CAN.viewBilling)
  const subscription = useGetSubscription(organisationId, deploymentId ?? null)
  const tiers = useGetTiers(organisationId)
  const account = useGetBillingAccount(organisationId, canViewBilling)
  const change = usePutSubscription()

  const changeTier = (tier: string) => {
    if (!organisationId || !deploymentId || change.isPending) return

    change.mutate({
      path: { organisation_id: organisationId, deployment_id: deploymentId },
      body: { tier },
    })
  }

  return (
    <PageDeploymentResources
      deployment={deployment.data?.data}
      isLoading={deployment.isLoading}
      subscription={{
        subscription: subscription.data,
        isLoading: subscription.isLoading,
        isError: subscription.isError,
        tiers: tiers.data?.data,
        access: { canManage, account: accountOf(canViewBilling, account) },
        changing: change.isPending ? (change.variables?.body.tier ?? null) : null,
        error: change.error?.message,
        onChange: changeTier,
      }}
    />
  )
}
