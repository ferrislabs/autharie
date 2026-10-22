import type { Schemas } from '@/api/api.client'

/**
 * Whether this installation can connect a cloud provider at all.
 *
 * False until the platform has said so: a tab drawn and then taken away is
 * worse than one drawn a moment late, and an installation that does not
 * answer -- an older control plane, a failed request -- is treated like one
 * that answered nothing.
 */
export function offersCloudProviders(
  response: Schemas.ListCloudProvidersResponse | null | undefined
): boolean {
  return (response?.data?.length ?? 0) > 0
}
