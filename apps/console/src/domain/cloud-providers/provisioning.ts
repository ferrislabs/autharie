import type { Schemas } from '@/api/api.client'

export type StepState = 'done' | 'current' | 'todo'

export type ProvisioningStepView = {
  id: Schemas.ProvisioningStep
  label: string
  state: StepState
}

export type ProvisioningNotice =
  | { kind: 'creating'; message: string; steps?: ProvisioningStepView[] }
  | { kind: 'failed'; message: string }

const STEPS: Array<{ id: Schemas.ProvisioningStep; label: string }> = [
  { id: 'creating_infrastructure', label: 'Creating the infrastructure' },
  { id: 'installing_data_plane', label: 'Installing the data plane' },
  { id: 'setting_up_iam', label: 'Setting up the IAM' },
]

export function stepsAt(current: Schemas.ProvisioningStep): ProvisioningStepView[] {
  const at = STEPS.findIndex((step) => step.id === current)
  return STEPS.map((step, index) => ({
    ...step,
    state: index < at ? 'done' : index === at ? 'current' : 'todo',
  }))
}

export const CREATING_MESSAGE = 'Creating your cluster'
export const FAILED_WITHOUT_REASON = 'The cluster could not be created.'

export function isCustomerCloud(distribution: Schemas.Distribution): boolean {
  return typeof distribution === 'object' && 'customer_cloud' in distribution
}

export function planeNotice(
  dataplane: Pick<Schemas.DataPlane, 'status' | 'failure_reason'> | undefined
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
  provisioning: Schemas.Provisioning | null | undefined
): ProvisioningNotice | null {
  if (!provisioning || !isCustomerCloud(deployment.distribution)) return null
  if (provisioning.status === 'provisioning') {
    if (!provisioning.step) return { kind: 'creating', message: CREATING_MESSAGE }
    const steps = stepsAt(provisioning.step)
    const current = steps.find((step) => step.state === 'current')
    return { kind: 'creating', message: current?.label ?? CREATING_MESSAGE, steps }
  }
  if (provisioning.status === 'failed') {
    return { kind: 'failed', message: provisioning.failure_reason ?? FAILED_WITHOUT_REASON }
  }
  return null
}

export function stillProvisioning(provisioning: Schemas.Provisioning | null | undefined): boolean {
  return provisioning?.status === 'provisioning'
}
