import { useState } from 'react'
import { ArrowLeft, Pencil, Trash2 } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { InfoRow, SectionPage } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Hint } from './hint'
import { DeliveriesTable } from './deliveries-table'
import { StateBadge } from './page-webhooks'
import { SecretPanel } from './secret-panel'

interface Props {
  isLoading: boolean
  error?: string
  webhook?: Schemas.WebhookView
  canManage: boolean
  isDeleting: boolean
  deleteError?: string
  secret: React.ComponentProps<typeof SecretPanel>
  deliveries: React.ComponentProps<typeof DeliveriesTable>
  onBack: () => void
  onEdit: () => void
  onDelete: () => void
}

export function PageWebhookDetail({
  isLoading,
  error,
  webhook,
  canManage,
  isDeleting,
  deleteError,
  secret,
  deliveries,
  onBack,
  onEdit,
  onDelete,
}: Props) {
  const [deleting, setDeleting] = useState(false)

  if (isLoading) return <Skeleton className='h-96 w-full' />

  if (!webhook) {
    return (
      <SectionPage
        title='Webhook'
        actions={
          <Button variant='ghost' size='sm' onClick={onBack}>
            <ArrowLeft className='h-3.5 w-3.5' />
            Back
          </Button>
        }
      >
        <p role='alert' className='text-sm text-destructive'>
          {error ?? 'This webhook does not exist. It may have been deleted.'}
        </p>
      </SectionPage>
    )
  }

  return (
    <SectionPage
      title={webhook.name}
      description={webhook.description ?? undefined}
      actions={
        <>
          <Button variant='ghost' size='sm' onClick={onBack}>
            <ArrowLeft className='h-3.5 w-3.5' />
            Webhooks
          </Button>
          {canManage && (
            <>
              <Button variant='outline' size='sm' onClick={onEdit}>
                <Pencil className='h-3.5 w-3.5' />
                Edit
              </Button>
              <Button variant='outline' size='sm' onClick={() => setDeleting(true)}>
                <Trash2 className='h-3.5 w-3.5' />
                Delete
              </Button>
            </>
          )}
        </>
      }
    >
      <div className='space-y-3 rounded-lg border bg-card p-5'>
        <InfoRow label='State' value={<StateBadge state={webhook.state} />} />
        <InfoRow label='Realm' value={webhook.realm} />
        <InfoRow
          label='Endpoint'
          value={<span className='break-all font-mono text-xs'>{webhook.endpoint}</span>}
        />
        <InfoRow
          label='Events'
          value={
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
          }
        />
        <InfoRow
          label='Custom headers'
          value={webhook.headers.length === 0 ? 'None' : webhook.headers.join(', ')}
        />
        <InfoRow label='Created' value={new Date(webhook.created_at).toLocaleString()} />
        <InfoRow label='Updated' value={new Date(webhook.updated_at).toLocaleString()} />
      </div>

      <SecretPanel {...secret} />

      <DeliveriesTable {...deliveries} />

      <Dialog open={deleting} onOpenChange={setDeleting}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {webhook.name}</DialogTitle>
            <DialogDescription>
              {webhook.name} stops receiving events from this deployment, and its signing secret go
              away. This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {deleteError && (
            <p role='alert' className='text-sm text-destructive'>
              {deleteError}
            </p>
          )}
          <DialogFooter>
            <Button variant='outline' onClick={() => setDeleting(false)}>
              Cancel
            </Button>
            <Button variant='destructive' disabled={isDeleting} onClick={onDelete}>
              {isDeleting && <Spinner className='size-3.5' />}
              Delete {webhook.name}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SectionPage>
  )
}
