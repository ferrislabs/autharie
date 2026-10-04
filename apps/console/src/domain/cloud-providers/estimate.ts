import { ApiRequestError } from '@/api/api.fetch'
import { formatEur } from './money'
import type { CostEstimate } from './types/cloud-provider'

export function describeEstimate(estimate: CostEstimate): string {
  const range =
    estimate.min === estimate.max
      ? `${formatEur(estimate.min)}`
      : `between ${formatEur(estimate.min)} and ${formatEur(estimate.max)}`

  return `${range} per month, billed by your cloud provider; egress not included`
}

export function estimateRefusal(error: unknown): string | undefined {
  if (error instanceof ApiRequestError && error.status === 422) return error.message
  return undefined
}
