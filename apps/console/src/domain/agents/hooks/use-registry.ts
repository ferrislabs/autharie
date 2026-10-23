import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import { REGISTRY_REFRESH_MS } from '../registry'
import {
  approveAgent,
  blockAgent,
  deleteAgent,
  getRegistry,
  setApprovalRequired,
  unblockAgent,
} from '../services/registry.service'
import type { Scope } from '../types'

function key(scope: Scope, ...rest: unknown[]) {
  return ['agents', scope.organisationId, scope.deploymentId, ...rest]
}

export function useAgentRegistry(scope: Scope) {
  const accessToken = useAuthStore(selectAccessToken)
  const enabled = !!scope.organisationId && !!scope.deploymentId && !!accessToken

  return useQuery({
    queryKey: key(scope, 'registry'),
    queryFn: () => getRegistry(scope),
    enabled,
    refetchInterval: REGISTRY_REFRESH_MS,
  })
}

export function useRegistryActions(scope: Scope) {
  const queryClient = useQueryClient()
  const refresh = () => queryClient.invalidateQueries({ queryKey: key(scope, 'registry') })

  const approvalRequired = useMutation({
    mutationFn: (required: boolean) => setApprovalRequired(scope, required),
    onSuccess: refresh,
  })
  const approve = useMutation({
    mutationFn: (clientId: string) => approveAgent(scope, clientId),
    onSuccess: refresh,
  })
  const block = useMutation({
    mutationFn: (clientId: string) => blockAgent(scope, clientId),
    onSuccess: refresh,
  })
  const unblock = useMutation({
    mutationFn: (clientId: string) => unblockAgent(scope, clientId),
    onSuccess: refresh,
  })
  const remove = useMutation({
    mutationFn: (clientId: string) => deleteAgent(scope, clientId),
    onSuccess: refresh,
  })

  return { approvalRequired, approve, block, unblock, remove, refresh }
}
