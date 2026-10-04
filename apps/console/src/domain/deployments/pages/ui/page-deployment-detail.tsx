import type { Schemas } from '@/api/api.client'
import { Card, EmptyState, Page, PageTitle, Section } from '@/components/layout/page'
import { Skeleton } from '@/components/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { formatDistanceToNow } from 'date-fns'
import { ProvisioningNoticeBanner } from '@/domain/cloud-providers/pages/ui/provisioning-notice'
import { describeDistribution } from '@/domain/cloud-providers/distribution'
import { provisioningNotice } from '@/domain/cloud-providers/provisioning'
import { KIND_LABELS } from '../../types/deployment'
import type { AvailabilityView } from '../../availability'
import { AvailabilityCard } from './components/availability-card'
import { DeploymentStatusBadge } from './components/deployment-status'

interface Props {
  deployment?: Schemas.Deployment
  actions: Schemas.Action[]
  availability: AvailabilityView
  dataplane?: Schemas.DataPlane
  isLoading: boolean
}

function actionStatusLabel(status: Schemas.ActionStatus): string {
  return typeof status === 'string' ? status : Object.keys(status)[0]
}

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <Card className='p-4'>
      <p className='text-sm text-muted-foreground'>{label}</p>
      <p className='mt-1 text-lg font-semibold'>{value}</p>
    </Card>
  )
}

export function PageDeploymentDetail({ deployment, actions, availability, dataplane, isLoading }: Props) {
  if (isLoading || !deployment) {
    return (
      <Page>
        <Skeleton className='h-9 w-64' />
        <Skeleton className='mt-8 h-40 w-full' />
      </Page>
    )
  }

  const notice = provisioningNotice(deployment, dataplane)

  return (
    <Page>
      <PageTitle
        title={deployment.name}
        badges={
          <>
            <DeploymentStatusBadge status={deployment.status} />
            <span className='text-xs text-muted-foreground'>
              {KIND_LABELS[deployment.kind]} · {deployment.version}
            </span>
          </>
        }
      />

      <div className='mt-8 space-y-8'>
        {notice && <ProvisioningNoticeBanner notice={notice} />}

        <div className='grid gap-4 sm:grid-cols-2 lg:grid-cols-4'>
          <Stat label='Version' value={<span className='font-mono'>{deployment.version}</span>} />
          <Stat label='Environment' value={deployment.environment} />
          <Stat label='Runs on' value={describeDistribution(deployment.distribution)} />
          <Stat
            label='Created'
            value={`${formatDistanceToNow(new Date(deployment.created_at))} ago`}
          />
        </div>

        <AvailabilityCard view={availability} />

        <Section title='Activity'>
          {actions.length === 0 ? (
            <EmptyState title='Nothing recorded yet' />
          ) : (
            <div className='overflow-x-auto rounded-lg border'>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Action</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className='text-right'>When</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {actions.map((action) => (
                    <TableRow key={action.id}>
                      <TableCell className='font-mono text-xs'>{action.action_type}</TableCell>
                      <TableCell className='text-muted-foreground'>
                        {actionStatusLabel(action.status)}
                      </TableCell>
                      <TableCell className='text-right text-xs text-muted-foreground'>
                        {formatDistanceToNow(new Date(action.metadata.created_at))} ago
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </Section>
      </div>
    </Page>
  )
}
