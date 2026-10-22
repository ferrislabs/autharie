import type { DeploymentStatus } from './types/deployment'

/**
 * What a person may stop: a deployment that has finished what it was doing and
 * is not being rewritten or taken away. One still coming up (`in_progress`,
 * created or started) has a task unfinished, and the platform refuses an order
 * until it is done. The platform is the one that decides; this only keeps the
 * menu from offering what it would refuse.
 */
const STOPPABLE: ReadonlySet<DeploymentStatus> = new Set<DeploymentStatus>([
  'successful',
  'failed',
  'maintenance',
  'upgrade_required',
])

export function canStop(status: DeploymentStatus | null | undefined): boolean {
  return !!status && STOPPABLE.has(status)
}

export function canStart(status: DeploymentStatus | null | undefined): boolean {
  return status === 'stopped'
}

export type PowerAction = 'stop' | 'start'

export interface PowerItem {
  action: PowerAction
  label: string
  disabled: boolean
}

export function powerItems(
  status: DeploymentStatus | null | undefined,
  busy: boolean,
): PowerItem[] {
  return [
    { action: 'stop', label: 'Stop deployment', disabled: busy || !canStop(status) },
    { action: 'start', label: 'Start deployment', disabled: busy || !canStart(status) },
  ]
}

export const STOP_CONFIRMATION = {
  title: 'Stop this deployment?',
  description:
    'It stops answering requests. Its data is kept, and it is still billed while it is stopped. You can start it again.',
  confirm: 'Stop deployment',
}
