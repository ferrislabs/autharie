import { useQuery } from '@tanstack/react-query'
import { loadCedar } from '../cedar-engine'

export function useCedar(enabled: boolean) {
  const query = useQuery({
    queryKey: ['cedar-wasm'],
    queryFn: loadCedar,
    enabled,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
    refetchOnWindowFocus: false,
  })

  return {
    api: query.data ?? null,
    isLoading: query.isLoading,
    failed: query.isError,
  }
}
