import { useNavigate } from '@tanstack/react-router'
import { useGetWebhooks } from '@/api/webhook.api'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
import { selectOrganisations, useOrganisationsStore } from '@/stores/organisations'
import { useWebhookScope } from '../../hooks/use-webhook-scope'
import { describeLimit } from '../../limit'
import { PageWebhooks } from '../ui/page-webhooks'

export default function PageWebhooksFeature() {
  const scope = useWebhookScope()
  const navigate = useNavigate()
  const deploymentPath = useDeploymentPath()
  const { can } = useMyPermissions()
  const organisations = useOrganisationsStore(selectOrganisations)

  const webhooks = useGetWebhooks(scope)
  const list = webhooks.data?.data ?? []
  const plan = organisations.find((item) => item.id === scope.organisationId)?.plan

  return (
    <PageWebhooks
      isLoading={webhooks.isLoading}
      error={webhooks.error?.message}
      webhooks={list}
      limit={describeLimit(list.length, plan)}
      canManage={can(CAN.manageInstances)}
      onCreate={() => void navigate({ to: deploymentPath('/identity/webhooks/new') })}
      onOpen={(webhookId) => void navigate({ to: deploymentPath(`/identity/webhooks/${webhookId}`) })}
    />
  )
}
