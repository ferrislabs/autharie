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

export function provisioningNotice(
  deployment: Pick<Schemas.Deployment, 'distribution'>,
  dataplane: Pick<Schemas.DataPlane, 'status' | 'failure_reason'> | undefined,
): ProvisioningNotice | null {
  return isCustomerCloud(deployment.distribution) ? planeNotice(dataplane) : null
}

export function stillProvisioning(
  dataplane: Pick<Schemas.DataPlane, 'status'> | undefined,
): boolean {
  return dataplane?.status === 'provisioning'
}
