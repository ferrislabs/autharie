import type { Schemas } from '@/api/api.client'

/**
 * Only a plane that is out of service can be removed: disabled by somebody, or
 * failed before it ever served. The server refuses anything else, this only
 * keeps the button from offering what it will refuse.
 */
export function canRemove(status: Schemas.DataPlaneStatus): boolean {
  return status === 'disabled' || status === 'failed'
}
