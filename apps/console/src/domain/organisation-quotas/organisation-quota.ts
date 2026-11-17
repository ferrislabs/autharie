import { describeUsage } from '@/domain/quotas/quota'
import type { OrganisationQuotaView } from './types'

export function heldLabel(quota: Pick<OrganisationQuotaView, 'held' | 'effective'>): string {
  return describeUsage(quota.held, quota.effective)
}

export function isOverLimit(quota: Pick<OrganisationQuotaView, 'held' | 'effective'>): boolean {
  return quota.effective.type === 'limited' && quota.held > quota.effective.max
}
