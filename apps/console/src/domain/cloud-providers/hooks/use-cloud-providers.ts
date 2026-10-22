import { useQuery } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'

const FIVE_MINUTES_MS = 5 * 60 * 1000

/**
 * The providers this installation can connect.
 *
 * It changes when the control plane restarts, not while somebody is looking,
 * so it is kept for a while rather than asked again on every focus.
 */
export const useCloudProviders = () => {
  const accessToken = useAuthStore(selectAccessToken)

  return useQuery({
    ...window.api.get('/cloud-providers').queryOptions,
    enabled: !!accessToken,
    staleTime: FIVE_MINUTES_MS,
  })
}
