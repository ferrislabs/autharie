import { Plus, Webhook } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState, SectionPage } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { endpointHost } from '../../form'
import { webhookBadge } from '../../state'
import type { WebhookLimit } from '../../limit'
import { Hint } from './hint'

export const SIGNATURE_NOTE = 'signed with HMAC-SHA256 in the x-ferriskey-signature header'

interface Props {
  isLoading: boolean
  error?: string
  webhooks: Schemas.WebhookView[]
  limit: WebhookLimit
  canManage: boolean
  onCreate: () => void
  onOpen: (webhookId: string) => void
}

export function PageWebhooks({
  isLoading,
  error,
  webhooks,
  limit,
  canManage,
  onCreate,
  onOpen,
}: Props) {
  if (isLoading) return <Skeleton className='h-64 w-full' />

  const createBlocked = !canManage || limit.atLimit
  const blockedReason = !canManage
    ? 'Creating a webhook needs the manage instances permission.'
    : limit.reason

  return (
    <SectionPage
      title='Webhooks'
      description='Send this deployment’s events to your own services as they happen.'
      actions={
        <>
          <span className='text-sm text-muted-foreground'>{limit.label}</span>
          {canManage && (
            <Button size='sm' onClick={onCreate} disabled={createBlocked}>
              <Plus className='h-3.5 w-3.5' />
              New webhook
            </Button>
          )}
        </>
      }
    >
      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      {limit.atLimit && <p className='text-sm text-muted-foreground'>{blockedReason}</p>}

      {webhooks.length === 0 ? (
        <EmptyState
          icon={<Webhook className='h-5 w-5' />}
          title='No webhooks yet'
          description={`A webhook tells your own service when something happens in this deployment, such as a user being created or a login failing. Each delivery is ${SIGNATURE_NOTE}, so your service can check it came from here.`}
          action={
            canManage ? (
              <Button onClick={onCreate}>
                <Plus className='h-3.5 w-3.5' />
                Create your first webhook
              </Button>
            ) : (
              <p className='text-sm text-muted-foreground'>
                Ask someone who can manage instances to create one.
              </p>
            )
          }
        />
      ) : (
        <div className='overflow-hidden rounded-lg border'>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Endpoint</TableHead>
                <TableHead>Realm</TableHead>
                <TableHead>Events</TableHead>
                <TableHead>State</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {webhooks.map((webhook) => (
                <WebhookRow key={webhook.id} webhook={webhook} onOpen={onOpen} />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </SectionPage>
  )
}

function WebhookRow({
  webhook,
  onOpen,
}: {
  webhook: Schemas.WebhookView
  onOpen: (webhookId: string) => void
}) {
  return (
    <TableRow>
      <TableCell className='font-medium'>
        <button
          type='button'
          className='text-left hover:underline'
          onClick={() => onOpen(webhook.id)}
        >
          {webhook.name}
        </button>
      </TableCell>
      <TableCell className='font-mono text-xs'>
        <Hint tip={webhook.endpoint}>{endpointHost(webhook.endpoint)}</Hint>
      </TableCell>
      <TableCell className='text-muted-foreground'>{webhook.realm}</TableCell>
      <TableCell>
        <Hint
          tip={
            <ul className='space-y-0.5'>
              {webhook.events.map((event) => (
                <li key={event} className='font-mono'>
                  {event}
                </li>
              ))}
            </ul>
          }
        >
          {webhook.events.length} {webhook.events.length === 1 ? 'event' : 'events'}
        </Hint>
      </TableCell>
      <TableCell>
        <StateBadge state={webhook.state} />
      </TableCell>
    </TableRow>
  )
}

export function StateBadge({ state }: { state: Schemas.WebhookState }) {
  const badge = webhookBadge(state)
  const content = <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>

  return badge.reason ? <Hint tip={badge.reason}>{content}</Hint> : content
}
