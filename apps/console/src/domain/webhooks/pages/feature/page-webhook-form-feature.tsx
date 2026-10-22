import { useMemo } from 'react'
import { useNavigate, useParams } from '@tanstack/react-router'
import {
  useCreateWebhook,
  useGetWebhook,
  useGetWebhookEvents,
  useGetWebhookRealms,
  useUpdateWebhook,
} from '@/api/webhook.api'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { emptyForm, formFromWebhook, toRequestBody, type WebhookForm } from '../../form'
import { useWebhookScope } from '../../hooks/use-webhook-scope'
import { PageWebhookForm } from '../ui/page-webhook-form'

export default function PageWebhookFormFeature() {
  const { webhookId } = useParams({ strict: false }) as { webhookId?: string }
  const scope = useWebhookScope()
  const navigate = useNavigate()
  const deploymentPath = useDeploymentPath()

  const editing = !!webhookId
  const webhook = useGetWebhook(scope, webhookId ?? null, () => false)
  const realms = useGetWebhookRealms(scope)
  const events = useGetWebhookEvents()
  const create = useCreateWebhook(scope)
  const update = useUpdateWebhook(scope)

  const realmList = useMemo(() => realms.data?.data ?? [], [realms.data])
  const initial = useMemo(
    () =>
      webhook.data?.data
        ? formFromWebhook(webhook.data.data)
        : emptyForm(realmList.includes('master') ? 'master' : (realmList[0] ?? '')),
    [webhook.data, realmList]
  )

  const back = () =>
    void navigate({
      to: deploymentPath(editing ? `/webhooks/${webhookId}` : '/webhooks'),
    })

  const submit = (form: WebhookForm) => {
    if (!scope.organisationId || !scope.deploymentId) return
    const path = { organisation_id: scope.organisationId, deployment_id: scope.deploymentId }
    const body = toRequestBody(form)

    if (webhookId) {
      update.mutate(
        { path: { ...path, webhook_id: webhookId }, body },
        { onSuccess: () => void navigate({ to: deploymentPath(`/webhooks/${webhookId}`) }) }
      )
    } else {
      create.mutate(
        { path, body },
        {
          onSuccess: async (response) => {
            const { data: created } = await response.json()
            void navigate({ to: deploymentPath(`/webhooks/${created.id}`) })
          },
        }
      )
    }
  }

  const mutation = editing ? update : create
  const loading = realms.isLoading || events.isLoading || (editing && webhook.isLoading)

  return (
    <PageWebhookForm
      key={`${webhookId ?? 'new'}:${realmList.join(',')}:${webhook.data?.data.updated_at ?? ''}`}
      mode={editing ? 'edit' : 'create'}
      isLoading={loading}
      initial={initial}
      realms={realmList}
      catalogue={events.data?.data ?? []}
      isSaving={mutation.isPending}
      error={mutation.error?.message ?? realms.error?.message ?? events.error?.message}
      onSubmit={submit}
      onCancel={back}
    />
  )
}
