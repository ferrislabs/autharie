import type { DeploymentStatus } from '../types/deployment'

export const FAILED_WITHOUT_REASON = 'The data plane did not say why it failed.'

export function failureNotice(
  status: DeploymentStatus,
  reason: string | null | undefined,
): string | undefined {
  if (status !== 'failed') return undefined
  return reason?.trim() ? reason : FAILED_WITHOUT_REASON
}
