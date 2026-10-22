import { ExternalLink, KeyRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { DeploymentStatusBadge } from '../pages/ui/components/deployment-status'
import type { DeploymentStatus } from '../types/deployment'
import { ActivityFeed, type ActivityEntry } from './activity-feed'
import { AvailabilityHistory, type AvailabilityDay } from './availability-history'
import { InstanceSummary, type InstanceFacts } from './instance-summary'
import { TrafficPanel, type TrafficMetric, type TrafficRange, type TrafficSeries } from './traffic-panel'

export interface PageDeploymentOverviewProps {
  name: string
  engine: string
  status: DeploymentStatus
  adminUrl?: string
  facts: InstanceFacts
  banner?: React.ReactNode
  traffic: TrafficSeries[]
  trafficLoading: boolean
  metric: TrafficMetric
  range: TrafficRange
  onMetricChange: (metric: TrafficMetric) => void
  onRangeChange: (range: TrafficRange) => void
  availability: {
    days: AvailabilityDay[]
    uptime: number | null
    probeIntervalSeconds: number
    isLoading: boolean
  }
  activity: ActivityEntry[]
  activityLoading: boolean
  clusterPanel?: React.ReactNode
}

const FRAME = 'mx-auto w-full max-w-6xl space-y-6 px-6 py-8'

export function PageDeploymentOverviewSkeleton() {
  return (
    <div className={FRAME} aria-busy>
      <div className='space-y-2'>
        <Skeleton className='h-8 w-64' />
        <Skeleton className='h-5 w-80' />
      </div>
      <div className='grid gap-6 lg:grid-cols-3'>
        <Skeleton className='h-96 lg:col-span-2' />
        <Skeleton className='h-96' />
        <Skeleton className='h-72 lg:col-span-2' />
        <Skeleton className='h-72' />
      </div>
    </div>
  )
}

export function PageDeploymentOverview(props: PageDeploymentOverviewProps) {
  const { facts, availability } = props

  return (
    <div className={FRAME}>
      {props.banner}

      <header className='flex flex-wrap items-start justify-between gap-4'>
        <div className='min-w-0 space-y-2'>
          <h1 className='truncate text-2xl font-semibold tracking-tight'>{props.name}</h1>
          <div className='flex flex-wrap items-center gap-2 text-sm text-muted-foreground'>
            <DeploymentStatusBadge status={props.status} />
            <span>{props.engine}</span>
            {facts.endpoint && (
              <>
                <span aria-hidden>·</span>
                <a
                  href={facts.endpoint}
                  target='_blank'
                  rel='noreferrer'
                  className='truncate underline-offset-4 hover:text-foreground hover:underline'
                >
                  {facts.endpoint.replace(/^https?:\/\//, '')}
                </a>
              </>
            )}
          </div>
        </div>
        <div className='flex shrink-0 items-center gap-2'>
          {facts.issuer && (
            <Button
              variant='outline'
              size='sm'
              onClick={() => void navigator.clipboard.writeText(facts.issuer ?? '')}
            >
              <KeyRound className='h-4 w-4' />
              Copy issuer
            </Button>
          )}
          {props.adminUrl && (
            <Button size='sm' asChild>
              <a href={props.adminUrl} target='_blank' rel='noreferrer'>
                Open admin console
                <ExternalLink className='h-4 w-4' />
              </a>
            </Button>
          )}
        </div>
      </header>

      <div className='grid gap-6 lg:grid-cols-3'>
        <div className='lg:col-span-2'>
          <TrafficPanel
            series={props.traffic}
            loading={props.trafficLoading}
            metric={props.metric}
            range={props.range}
            onMetricChange={props.onMetricChange}
            onRangeChange={props.onRangeChange}
          />
        </div>
        <InstanceSummary facts={facts} />
        <div className='lg:col-span-2'>
          {availability.isLoading ? (
            <Skeleton className='h-72 w-full' />
          ) : (
            <AvailabilityHistory
              days={availability.days}
              uptime={availability.uptime}
              probeIntervalSeconds={availability.probeIntervalSeconds}
            />
          )}
        </div>
        {props.activityLoading ? (
          <Skeleton className='h-72 w-full' />
        ) : (
          <ActivityFeed entries={props.activity} />
        )}
      </div>

      {props.clusterPanel}
    </div>
  )
}
