import type { Schemas } from '@/api/api.client'

export type ProvisioningNotice =
  | { kind: 'creating'; message: string }
  | { kind: 'failed'; message: string }

export const CREATING_MESSAGE = 'Creating your cluster'
export const FAILED_WITHOUT_REASON = 'The cluster could not be created.'

export function isCustomerCloud(distribution: Schemas.Distribution): boolean {
  return typeof distribution === 'object' && 'customer_cloud' in distribution
}

export function planeNotice(
  dataplane: Pick<Schemas.DataPlane, 'status' | 'failure_reason'> | undefined,
): ProvisioningNotice | null {
  if (!dataplane) return null
  if (dataplane.status === 'provisioning') return { kind: 'creating', message: CREATING_MESSAGE }
  if (dataplane.status === 'failed') {
    return { kind: 'failed', message: dataplane.failure_reason ?? FAILED_WITHOUT_REASON }
  }
  return null
}

export const PROVISIONING_POLL_MS = 5000

export function provisioningNotice(
  deployment: Pick<Schemas.Deployment, 'distribution'>,
  provisioning: Schemas.Provisioning | null | undefined,
): ProvisioningNotice | null {
  if (!provisioning || !isCustomerCloud(deployment.distribution)) return null
  if (provisioning.status === 'provisioning') return { kind: 'creating', message: CREATING_MESSAGE }
  if (provisioning.status === 'failed') {
    return { kind: 'failed', message: provisioning.failure_reason ?? FAILED_WITHOUT_REASON }
  }
  return null
}

export function stillProvisioning(provisioning: Schemas.Provisioning | null | undefined): boolean {
  return provisioning?.status === 'provisioning'
}
