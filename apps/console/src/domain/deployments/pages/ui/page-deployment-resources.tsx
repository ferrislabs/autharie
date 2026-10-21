import type { Schemas } from '@/api/api.client'
import { SectionPage } from '@/components/layout/page'
import { Card } from '@/components/layout/page'
import { Skeleton } from '@/components/ui/skeleton'
import { hostingOf } from '@/domain/billing/billing'
import { SubscriptionCard, type SubscriptionCardProps } from './components/subscription-card'
import { formatCpu, formatMemory, formatStorage } from '../../types/resources'

interface Props {
  deployment?: Schemas.Deployment
  isLoading: boolean
  subscription: Omit<SubscriptionCardProps, 'fit'>
}

export function PageDeploymentResources({ deployment, isLoading, subscription }: Props) {
  if (isLoading || !deployment) {
    return <Skeleton className='h-40 w-full' />
  }

  const { cpu_millis, memory_mib, storage_gib } = deployment.resources

  return (
    <SectionPage
      title='Resources'
      description='What this instance was given when it was created.'
    >
      <SubscriptionCard
        {...subscription}
        fit={{ engine: deployment.kind, hosting: hostingOf(deployment.distribution) }}
      />

      <div className='grid gap-4 sm:grid-cols-3'>
        <Card className='p-4'>
          <p className='text-sm text-muted-foreground'>CPU</p>
          <p className='mt-1 text-2xl font-semibold tabular-nums'>{formatCpu(cpu_millis)}</p>
        </Card>
        <Card className='p-4'>
          <p className='text-sm text-muted-foreground'>Memory</p>
          <p className='mt-1 text-2xl font-semibold tabular-nums'>{formatMemory(memory_mib)}</p>
        </Card>
        <Card className='p-4'>
          <p className='text-sm text-muted-foreground'>Storage</p>
          <p className='mt-1 text-2xl font-semibold tabular-nums'>{formatStorage(storage_gib)}</p>
        </Card>
      </div>

      <p className='text-sm text-muted-foreground'>
        Changing these means moving the instance, so it is not something to do from a form
        without saying what it will cost in downtime. It is not offered yet.
      </p>
    </SectionPage>
  )
}
