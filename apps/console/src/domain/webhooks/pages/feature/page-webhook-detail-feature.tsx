import { useState } from 'react'
import { useNavigate, useParams } from '@tanstack/react-router'
import {
  useDeleteWebhook,
  useGetWebhook,
  useGetWebhookDeliveries,
  useRetryWebhookDelivery,
  useRevealWebhookSecret,
  useRotateWebhookSecret,
} from '@/api/webhook.api'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
import { useTickingClock } from '@/hooks/use-ticking-clock'
import type { Schemas } from '@/api/api.client'
import { SECRET_POLL_MS, secretPanel, shouldPollSecret, type Rotation } from '../../secret'
import { useWebhookScope } from '../../hooks/use-webhook-scope'
import { PageWebhookDetail } from '../ui/page-webhook-detail'

export default function PageWebhookDetailFeature() {
  const { webhookId } = useParams({ strict: false }) as { webhookId?: string }
  const scope = useWebhookScope()
  const navigate = useNavigate()
  const deploymentPath = useDeploymentPath()
  const { can } = useMyPermissions()
  const canManage = can(CAN.manageInstances)
  const now = useTickingClock(1000)

  const [watchedSince] = useState(() => Date.now())
  const [rotation, setRotation] = useState<Rotation | null>(null)
  const [revealed, setRevealed] = useState<{ secret: string; version: number } | null>(null)
  const [filter, setFilter] = useState('all')
  const [requested, setRequested] = useState<ReadonlySet<string>>(new Set())
  const [retryError, setRetryError] = useState<string>()

  const webhook = useGetWebhook(scope, webhookId ?? null, (data?: Schemas.WebhookView) => {
    if (!data) return false
    const waiting = shouldPollSecret(secretPanel(data, rotation, Date.now(), watchedSince))
    return waiting || data.state.phase === 'Pending' ? SECRET_POLL_MS : false
  })
  const deliveries = useGetWebhookDeliveries(
    scope,
    webhookId ?? null,
    filter === 'all' ? undefined : filter
  )
  const reveal = useRevealWebhookSecret()
  const rotate = useRotateWebhookSecret(scope)
  const retry = useRetryWebhookDelivery()
  const remove = useDeleteWebhook(scope)

  const data = webhook.data?.data
  const panel = data ? secretPanel(data, rotation, now, watchedSince) : null
  const shownSecret =
    panel?.kind === 'ready' && revealed?.version === panel.version ? revealed.secret : null

  const path =
    scope.organisationId && scope.deploymentId && webhookId
      ? {
          organisation_id: scope.organisationId,
          deployment_id: scope.deploymentId,
          webhook_id: webhookId,
        }
      : null

  const goToList = () => void navigate({ to: deploymentPath('/identity/webhooks') })

  return (
    <PageWebhookDetail
      isLoading={webhook.isLoading}
      error={webhook.error?.message}
      webhook={data}
      canManage={canManage}
      isDeleting={remove.isPending}
      deleteError={remove.error?.message}
      secret={{
        panel: panel ?? { kind: 'generating' },
        revealed: shownSecret,
        isRevealing: reveal.isPending,
        isRotating: rotate.isPending,
        canManage,
        error: reveal.error?.message ?? rotate.error?.message,
        onReveal: () => {
          if (!path) return
          reveal.mutate({ path }, { onSuccess: (result) => setRevealed(result.data) })
        },
        onHide: () => setRevealed(null),
        onRotate: () => {
          if (!path || !data) return
          setRevealed(null)
          rotate.mutate(
            { path },
            {
              onSuccess: async (response) => {
                const { data: request } = await response.json()
                setRotation({
                  requestId: request.id,
                  startedAt: Date.now(),
                  versionBefore: data.secret_version ?? null,
                })
              },
            }
          )
        },
      }}
      deliveries={{
        isLoading: deliveries.isLoading,
        error: deliveries.error?.message ?? retryError,
        deliveries: deliveries.data?.data ?? [],
        filter,
        onFilter: setFilter,
        requested,
        canManage,
        now,
        onRetry: (deliveryId) => {
          if (!path) return
          setRetryError(undefined)
          setRequested((current) => new Set(current).add(deliveryId))
          retry.mutate(
            { path: { ...path, delivery_id: deliveryId } },
            {
              onError: (error) => {
                setRequested((current) => {
                  const next = new Set(current)
                  next.delete(deliveryId)
                  return next
                })
                setRetryError(error.message)
              },
            }
          )
        },
      }}
      onBack={goToList}
      onEdit={() => void navigate({ to: deploymentPath(`/identity/webhooks/${webhookId}/edit`) })}
      onDelete={() => {
        if (!path) return
        remove.mutate({ path }, { onSuccess: goToList })
      }}
    />
  )
}
