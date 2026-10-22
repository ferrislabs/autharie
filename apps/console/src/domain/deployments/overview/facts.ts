import type { Schemas } from '@/api/api.client'
import { OFFER_COPY } from '../offers'
import { nextUpgrade } from '@/domain/upgrades/version'

export function formatResources(resources: Schemas.DeploymentResources): string {
  const cores = Number((resources.cpu_millis / 1000).toFixed(2))
  const memory =
    resources.memory_mib % 1024 === 0
      ? `${resources.memory_mib / 1024} GiB`
      : `${resources.memory_mib} MiB`

  return `${cores} vCPU · ${memory} · ${resources.storage_gib} GiB database`
}

export function endpointOf(hostname: string | undefined): string | undefined {
  return hostname ? `https://${hostname}` : undefined
}

export function issuerOf(
  endpoint: string | undefined,
  kind: Schemas.DeploymentKind,
): string | undefined {
  if (!endpoint) return undefined
  return kind === 'ferriskey' ? `${endpoint}/api/realms/master` : `${endpoint}/realms/master`
}

export function adminUrlOf(
  endpoint: string | undefined,
  kind: Schemas.DeploymentKind,
): string | undefined {
  return endpoint && kind === 'ferriskey' ? `${endpoint}/admin` : undefined
}

export function offerLabel(offer: Schemas.Offer | null | undefined): string {
  return offer ? OFFER_COPY[offer].label : 'Custom'
}

export function latestVersionOf(
  current: string,
  releases: Schemas.ReleaseAvailability[],
): string | undefined {
  return nextUpgrade(current, releases)?.id.version
}
