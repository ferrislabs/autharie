import { useMemo, useState } from 'react'
import { useParams } from '@tanstack/react-router'
import {
  useGetDeployment,
  useGetDeploymentActions,
  useStartDeployment,
  useStopDeployment,
} from '@/api/deployment.api'
import { useGetDeploymentAvailability } from '@/api/deployment-availability.api'
import { useGetDeploymentUsage } from '@/api/metrics.api'
import { useGetReleaseAvailability } from '@/api/upgrade.api'
import { ClusterPanelFeature } from '@/domain/cloud-providers/pages/feature/cluster-panel-feature'
import { describeDistribution } from '@/domain/cloud-providers/distribution'
import { ProvisioningNoticeBanner } from '@/domain/cloud-providers/pages/ui/provisioning-notice'
import { provisioningNotice } from '@/domain/cloud-providers/provisioning'
import {
  currentCredentialId,
  currentProfile,
  isResizable,
} from '@/domain/cloud-providers/resize'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { CAN } from '@/domain/organisations/permissions'
import type { MetricKey } from '@/domain/usage/series'
import { toActivity } from '../../overview/activity'
import { toAvailabilityDays, unobservedDays } from '../../overview/availability'
import {
  adminUrlOf,
  endpointOf,
  formatResources,
  issuerOf,
  latestVersionOf,
  offerLabel,
} from '../../overview/facts'
import {
  PageDeploymentOverview,
  PageDeploymentOverviewSkeleton,
} from '../../overview/page-deployment-overview'
import type { TrafficMetric, TrafficRange } from '../../overview/traffic-panel'
import { previousWindow, seriesFor, windowFor, type Window } from '../../overview/traffic'
import { KIND_LABELS } from '../../types/deployment'

const METRICS: MetricKey[] = ['requests', 'logins', 'active_users', 'token_events']

function useUsage(
  organisationId: string | null,
  deploymentId: string | null,
  metric: MetricKey,
  span: Window,
) {
  return useGetDeploymentUsage(
    organisationId,
    deploymentId,
    metric,
    new Date(span.from).toISOString(),
    new Date(span.until).toISOString(),
  )
}

export default function PageDeploymentOverviewFeature() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }
  const organisationId = useResolvedOrganisationId() ?? null
  const id = deploymentId ?? null

  const [metric, setMetric] = useState<TrafficMetric>('requests')
  const [range, setRange] = useState<TrafficRange>('24h')
  const [span, setSpan] = useState(() => windowFor('24h', Date.now()))

  const { can } = useMyPermissions()
  const stop = useStopDeployment()
  const start = useStartDeployment()

  const deployment = useGetDeployment(id)
  const actions = useGetDeploymentActions(id)
  const availability = useGetDeploymentAvailability(organisationId, id)
  const releases = useGetReleaseAvailability(organisationId, id)

  const requests = useUsage(organisationId, id, 'requests', span)
  const logins = useUsage(organisationId, id, 'logins', span)
  const activeUsers = useUsage(organisationId, id, 'active_users', span)
  const tokenEvents = useUsage(organisationId, id, 'token_events', span)
  const previous = useUsage(organisationId, id, metric, previousWindow(span))

  const traffic = useMemo(() => {
    const current = {
      requests: requests.data?.data,
      logins: logins.data?.data,
      active_users: activeUsers.data?.data,
      token_events: tokenEvents.data?.data,
    }

    return METRICS.map((key) =>
      seriesFor(
        key,
        current[key] ?? [],
        span,
        range,
        key === metric ? previous.data?.data : undefined,
      ),
    )
  }, [
    requests.data,
    logins.data,
    activeUsers.data,
    tokenEvents.data,
    previous.data,
    span,
    range,
    metric,
  ])

  const entries = useMemo(() => toActivity(actions.data?.data ?? []), [actions.data])

  const days = useMemo(
    () =>
      availability.data
        ? toAvailabilityDays(availability.data.data.days)
        : unobservedDays(span.until),
    [availability.data, span.until],
  )

  const subject = deployment.data?.data
  if (deployment.isLoading || !subject) return <PageDeploymentOverviewSkeleton />

  const hostname = subject.hostname ?? undefined
  const endpoint = endpointOf(hostname)
  const notice = provisioningNotice(subject, deployment.data?.provisioning)

  const profile = currentProfile(subject.distribution)
  const credentialId = currentCredentialId(subject.distribution)
  const resizable =
    profile && credentialId && isResizable(subject, deployment.data?.provisioning)

  return (
    <PageDeploymentOverview
      name={subject.name}
      engine={KIND_LABELS[subject.kind]}
      status={subject.status}
      adminUrl={adminUrlOf(endpoint, subject.kind)}
      banner={notice && <ProvisioningNoticeBanner notice={notice} />}
      power={
        can(CAN.manageInstances)
          ? {
              status: subject.status,
              busy: stop.isPending || start.isPending,
              refusal: stop.error?.message ?? start.error?.message,
              onStop: () => {
                if (!organisationId) return
                start.reset()
                stop.mutate({ path: { organisation_id: organisationId, deployment_id: subject.id } })
              },
              onStart: () => {
                if (!organisationId) return
                stop.reset()
                start.mutate({ path: { organisation_id: organisationId, deployment_id: subject.id } })
              },
            }
          : undefined
      }
      facts={{
        kind: subject.kind,
        endpoint,
        issuer: issuerOf(endpoint, subject.kind),
        version: subject.version,
        latestVersion: latestVersionOf(subject.version, releases.data?.data ?? []),
        offer: offerLabel(subject.offer),
        resources: formatResources(subject.resources),
        runsOn: describeDistribution(subject.distribution),
        createdAt: Date.parse(subject.created_at),
      }}
      traffic={traffic}
      trafficLoading={requests.isLoading}
      metric={metric}
      range={range}
      onMetricChange={setMetric}
      onRangeChange={(next) => {
        setRange(next)
        setSpan(windowFor(next, Date.now()))
      }}
      availability={{
        days,
        uptime: availability.data?.data.uptime ?? null,
        probeIntervalSeconds: availability.data?.data.probe_interval_seconds ?? 60,
        isLoading: availability.isLoading,
      }}
      activity={entries}
      activityLoading={actions.isLoading}
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
    />
  )
}
